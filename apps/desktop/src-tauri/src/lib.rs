mod accept_loop;
mod agent_commands;
/// Public so the live harness binaries resolve the agent exactly as the app
/// does. Duplicating the token/manifest choice is how the two drift apart.
pub mod agent_runtime;
mod android_tools;
mod command_error;
mod commands;
mod farm_commands;
mod flow_commands;
mod idle_sweeper;
mod interaction_commands;
pub mod interaction_ocr;
mod local_api;
mod nurture_commands;
mod nurture_schedule;
mod peripherals;
mod publish_commands;
mod state;
mod view_hub;
mod view_watchdog;

use crate::command_error::CommandError;
use state::AppState;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// How much log one file holds before it is rotated, and how many are kept.
///
/// Named constants rather than literals at the call site so the test below can assert the
/// property that matters — that neither of them is the plugin's default — instead of
/// re-stating the numbers.
const LOG_FILE_BYTES: u128 = 8 * 1024 * 1024;
const LOG_FILES_KEPT: usize = 5;

/// The plugin's own defaults, restated so a change to them is visible from here.
///
/// From `tauri-plugin-log` 2.9: `DEFAULT_MAX_FILE_SIZE = 40_000` with
/// `DEFAULT_ROTATION_STRATEGY = KeepOne`.
const PLUGIN_DEFAULT_MAX_FILE_SIZE: u128 = 40_000;

/// The rules the two constants above have to satisfy, checked at compile time.
///
/// A `#[test]` was the first shape of this and clippy was right to refuse it: these are
/// constants, so the question is answerable while the crate is being built, and a compile
/// error is a stronger guarantee than a test somebody has to run. Anyone who drops the size
/// back toward the plugin's 40 KB — the value that made the log useless — will not get a
/// binary out of it.
const _: () = {
    assert!(
        LOG_FILE_BYTES > PLUGIN_DEFAULT_MAX_FILE_SIZE,
        "40 KB of Warn output is minutes on a twenty-device farm; that default is the bug"
    );
    // A working day of Warn output with room for a burst...
    assert!(LOG_FILE_BYTES >= 4 * 1024 * 1024);
    // ...and a worst case that is still trivial on a machine driving twenty phones.
    assert!(LOG_FILE_BYTES * LOG_FILES_KEPT as u128 <= 64 * 1024 * 1024);
    // More than one file: the rotation has to leave something behind, which is the half
    // `KeepOne` got wrong -- it deletes rather than archives.
    assert!(LOG_FILES_KEPT > 1);
};

/// Why the app could not start, if it could not — and the lock that lets it try again.
///
/// The message used to be a plain `Option<String>` fixed at setup, which is what made the
/// startup screen's only button useless: it called `window.location.reload()`, the WebView
/// came back, asked `startup_error` again and was handed the same stored sentence.
/// `AppState::bootstrap` had run once and would never run again, so the operator could fix
/// whatever was wrong — plug in adb, start the sidecar — and had no way to tell the app.
/// The only real remedy was quitting and reopening.
#[derive(Default)]
struct StartupState {
    error: parking_lot::Mutex<Option<String>>,
    /// Serialises retries, so two impatient clicks cannot bootstrap twice.
    attempt: tokio::sync::Mutex<()>,
}

#[tauri::command]
fn startup_error(state: tauri::State<'_, StartupState>) -> Option<String> {
    state.error.lock().clone()
}

/// Try the bootstrap again, and answer with whatever is wrong *now*.
///
/// `None` means the app is up: either this attempt succeeded, or another one already did.
/// A second success is impossible rather than merely unlikely — `Manager::manage` refuses a
/// type that is already managed, and a bootstrap that ran twice would leave two of every
/// background task running against one database.
#[tauri::command]
async fn retry_startup(
    app: tauri::AppHandle,
    state: tauri::State<'_, StartupState>,
) -> Result<Option<String>, CommandError> {
    let _attempt = state.attempt.lock().await;
    if app.try_state::<AppState>().is_some() {
        *state.error.lock() = None;
        return Ok(None);
    }
    let resource_dir = app.path().resource_dir().ok();
    match AppState::bootstrap(resource_dir).await {
        Ok(fresh) => {
            fresh.spawn_background_tasks(app.clone());
            if !app.manage(fresh) {
                // Lost a race this lock exists to prevent. Whoever won is the live state.
                log::warn!("a concurrent startup retry had already installed the app state");
            }
            *state.error.lock() = None;
            Ok(None)
        }
        Err(error) => {
            let message = format!("{error:#}");
            log::error!("desktop startup retry is still blocked: {message}");
            *state.error.lock() = Some(message.clone());
            Ok(Some(message))
        }
    }
}

/// The message a panic payload carries, whatever shape it was thrown in.
///
/// `panic!("text")` yields `&'static str`, `panic!("{x}")` yields `String`, and `panic_any`
/// yields anything at all. The third shape is the one that would otherwise read as an empty
/// log line -- a record that says a panic happened and nothing about it.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        return text;
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.as_str();
    }
    "a panic payload that is neither &str nor String"
}

/// What a panic leaves behind, as one line.
///
/// **Pure, and deliberately separate from the hook that uses it.** The hook itself runs while
/// the process is already dying and cannot be exercised by a test without taking the test
/// runner with it, so the part worth pinning -- that the message, the location and the thread
/// all reach the text -- lives here where a test can read it.
fn panic_report(message: &str, location: Option<&str>, thread: &str) -> String {
    format!(
        "PANIC in thread '{thread}' at {}: {message} (the process aborts here, so this line is          the whole record)",
        location.unwrap_or("an unknown location"),
    )
}

/// **Write the panic down before the process dies, because it dies immediately.**
///
/// The release profile sets `panic = "abort"`, and that has a consequence which is easy to
/// miss: **tokio's per-task panic isolation does not apply.** A panic inside a spawned task
/// never becomes a `JoinError` for anyone to handle -- it takes the whole process with it. The
/// tasks in question are the ones every phone depends on: the scrcpy reader, the device cleanup
/// worker, the job queue, the Flow runtime, the view-hub accept loop. One panic in any of them
/// ends the work on *every* phone at once, mid-campaign.
///
/// The profile also sets `strip = "symbols"`, so there is no usable backtrace to recover
/// afterwards. Before this hook existed the entire operator-visible record of a panic was **a
/// window that vanished** -- which is the report that started this work. The device-control path
/// alone rests on more than thirty `expect()` calls asserting lease and reservation invariants;
/// every one of them is a candidate, and each was silent.
///
/// Two details that make the line actually survive:
///
/// * It is installed **first** in `run()`, ahead of `install_process_tree_guard`, which itself
///   `expect()`s. A hook installed after the first thing that can panic does not cover it.
/// * `log::error!` reaches the rotating file the log plugin owns, and `std::fs::File` writes are
///   unbuffered syscalls rather than buffered ones -- so the record is already on disk when the
///   abort follows, with no flush to miss. The default hook still runs afterwards, which keeps
///   the familiar stderr message for anyone running from a terminal.
///
/// Note the log plugin is registered inside `.setup()`, so a panic *before* that point has
/// nowhere to write. That window is startup only, and the default hook still covers stderr.
fn install_panic_logging() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info.location().map(|location| location.to_string());
        let report = panic_report(
            panic_message(info.payload()),
            location.as_deref(),
            thread.name().unwrap_or("unnamed"),
        );
        log::error!("{report}");
        default_hook(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // First, before anything that can panic -- including the `expect` on the next line.
    install_panic_logging();
    riviu_ios_driver::install_process_tree_guard()
        .expect("failed to establish process-tree ownership");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Registered, but nothing checks on its own. A farm machine is often offline and
        // nobody asked it to phone home at startup, so the check is an explicit operator
        // action — see `update_check`.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let window = if let Some(window) = app.get_webview_window("main") {
                window
            } else {
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Riviu Manager")
                    .inner_size(1440.0, 900.0)
                    .min_inner_size(1100.0, 700.0)
                    .resizable(true)
                    .visible(true)
                    .build()?
            };
            window.show()?;
            window.set_focus()?;
            // Registered in release too, at Warn. It used to be debug-only, which meant
            // an operator hitting a driver failure had no record of it anywhere -- and
            // the driver's warnings are exactly the ones worth keeping: a scrcpy server
            // that ignored SIGTERM, a reclaimed leaked forward, a producer restart.
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(if cfg!(debug_assertions) {
                        log::LevelFilter::Info
                    } else {
                        log::LevelFilter::Warn
                    })
                    // **The defaults threw the log away, which is worse than not writing
                    // one.** `tauri-plugin-log` ships `max_file_size = 40_000` bytes with
                    // `RotationStrategy::KeepOne`, and `KeepOne` *deletes* the file when it
                    // rotates rather than archiving it. Forty kilobytes is a few hundred
                    // lines; on a twenty-device farm that is minutes. Opening the log after
                    // an incident showed the seconds *after* the incident and nothing else,
                    // and the whole reason release logging was turned on was to have a
                    // record of driver failures — a scrcpy server that ignored SIGTERM, a
                    // reclaimed leaked forward, a producer restart.
                    //
                    // Five files of 8 MB is 40 MB at worst on a machine that runs a device
                    // farm, and it covers a full working day of Warn-level output with room
                    // for a burst. `KeepSome` archives with a date in the name, so the file
                    // an operator is asked for still exists an hour later.
                    .max_file_size(LOG_FILE_BYTES)
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(LOG_FILES_KEPT))
                    .build(),
            )?;

            let handle = app.handle().clone();
            let resource_dir = app.path().resource_dir().ok();
            let startup_state =
                match tauri::async_runtime::block_on(AppState::bootstrap(resource_dir)) {
                    Ok(state) => {
                        state.spawn_background_tasks(handle.clone());
                        handle.manage(state);
                        StartupState::default()
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        log::error!("desktop startup is blocked: {message}");
                        StartupState {
                            error: parking_lot::Mutex::new(Some(message)),
                            ..Default::default()
                        }
                    }
                };

            handle.manage(startup_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup_error,
            retry_startup,
            agent_commands::agent_get_settings,
            agent_commands::agent_save_settings,
            agent_commands::agent_list_statuses,
            agent_commands::agent_preflight,
            agent_commands::agent_repair,
            agent_commands::agent_bulk_repair,
            commands::list_devices,
            commands::refresh_devices,
            commands::prepare_device,
            commands::install_ipa,
            commands::install_ipa_to_group,
            commands::install_unsigned_ipa,
            commands::uninstall_app,
            commands::list_installed_apps,
            commands::device_shell,
            commands::import_media,
            commands::export_media,
            commands::set_screen_rotation,
            commands::screenshot,
            commands::syslog,
            commands::reboot_device,
            commands::backup_device,
            commands::restore_device,
            commands::device_tap,
            commands::device_swipe,
            commands::device_swipe_path,
            commands::device_type_text,
            commands::device_home,
            commands::device_key,
            commands::set_screen_locked,
            local_api::local_api_get_config,
            local_api::local_api_set_config,
            peripherals::list_serial_ports,
            peripherals::relay_set_channel,
            peripherals::relay_pulse_channel,
            commands::device_control_begin,
            commands::device_control_end,
            commands::group_input,
            commands::distribute_text,
            commands::distribute_files,
            commands::enable_wifi_adb,
            commands::disable_wifi_adb,
            commands::wifi_adb_connect,
            commands::wifi_adb_disconnect,
            commands::arp_scan,
            commands::set_wallpaper,
            commands::set_wallpaper_bytes,
            commands::set_mock_location,
            commands::stop_mock_location,
            commands::is_rooted,
            commands::set_device_identity,
            commands::factory_reset,
            commands::root_shell,
            // The per-phone function menu (xiaowei 功能), one command per row.
            commands::device_list_dir,
            commands::device_pull_path,
            commands::device_push_file,
            commands::device_delete_path,
            commands::set_wifi_radio,
            commands::reset_display_metrics,
            commands::power_off_device,
            commands::open_system_settings,
            commands::wake_screen,
            commands::screenshot_to_device,
            commands::set_input_method,
            commands::launch_device_app,
            commands::device_get_clipboard,
            commands::device_set_clipboard,
            commands::get_stream_settings,
            commands::set_stream_settings,
            commands::latest_frame,
            commands::view_endpoint,
            commands::view_report_paint,
            commands::view_ensure,
            commands::view_request_keyframe,
            commands::view_inject_touch,
            commands::view_set_preset,
            commands::save_view_snapshot,
            commands::list_jobs,
            commands::run_script,
            commands::cancel_job,
            commands::list_scripts,
            commands::save_script,
            commands::example_script,
            commands::get_apple_id,
            commands::set_apple_id,
            commands::clear_apple_id,
            commands::resign_wda,
            commands::bulk_resign_wda,
            commands::driver_mode,
            commands::driver_degraded_reason,
            commands::android_unavailable_reason,
            commands::android_tool_problems,
            commands::log_frontend_error,
            commands::update_check,
            commands::update_install,
            farm_commands::get_device_meta,
            farm_commands::list_device_metas,
            farm_commands::save_device_meta,
            farm_commands::list_groups,
            farm_commands::save_group,
            farm_commands::delete_group,
            farm_commands::list_materials,
            farm_commands::add_material,
            farm_commands::delete_material,
            farm_commands::push_material,
            farm_commands::list_apps_library,
            farm_commands::add_app_library,
            farm_commands::delete_app_library,
            farm_commands::install_library_app,
            farm_commands::list_schedules,
            farm_commands::save_schedule,
            farm_commands::delete_schedule,
            farm_commands::list_op_logs,
            farm_commands::analytics_summary,
            farm_commands::api_docs,
            flow_commands::flow_action_catalog,
            flow_commands::flow_list,
            flow_commands::flow_get,
            flow_commands::flow_validate,
            flow_commands::flow_save_revision,
            flow_commands::flow_archive,
            flow_commands::flow_import_legacy,
            flow_commands::flow_export,
            flow_commands::flow_run,
            flow_commands::flow_cancel_run,
            flow_commands::flow_retry_attempt,
            flow_commands::flow_list_runs,
            flow_commands::flow_get_run,
            flow_commands::flow_coordinate_frame,
            flow_commands::flow_read_artifact,
            interaction_commands::interaction_parse_links,
            interaction_commands::interaction_resolve_links,
            interaction_commands::interaction_preview_thread,
            interaction_commands::interaction_measure_post,
            interaction_commands::interaction_start_thread,
            interaction_commands::interaction_list,
            interaction_commands::interaction_get,
            interaction_commands::interaction_cancel,
            interaction_commands::interaction_retry,
            interaction_commands::interaction_open_on_device,
            interaction_commands::interaction_list_target_notes,
            interaction_commands::interaction_list_artifacts,
            interaction_commands::interaction_read_artifact,
            nurture_commands::nurture_get_settings,
            nurture_commands::nurture_save_settings,
            nurture_commands::nurture_test_api,
            nurture_commands::nurture_list_comment_attempts,
            nurture_commands::nurture_cost_summary,
            nurture_commands::nurture_session_status,
            nurture_commands::nurture_session_log,
            nurture_commands::nurture_session_log_summary,
            nurture_commands::nurture_clear_session_log,
            nurture_commands::nurture_start,
            nurture_commands::nurture_stop,
            publish_commands::publish_scan_folder,
            publish_commands::publish_create_campaign,
            publish_commands::publish_list,
            publish_commands::publish_get,
            publish_commands::publish_cancel,
            publish_commands::publish_prepare,
            publish_commands::publish_transfer,
            publish_commands::publish_post,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        // Two events, one sequence, and both are needed.
        //
        // `ExitRequested` fires when something *asks* the app to quit — which is the path
        // an updater takes before handing over to the installer, and the only one it is
        // guaranteed to reach. `Exit` fires on the way out for a normal quit. Wiring only
        // `Exit` is what left the sequence skippable: an updater that calls `process::exit`
        // after requesting the quit never reaches it, and every phone keeps its WDA relay,
        // its XCTest runner and its adb forward.
        //
        // Calling it twice on a normal quit is harmless — see `graceful_shutdown`, every
        // step is idempotent — and a doubled cleanup is a far better failure than a
        // skipped one.
        if !matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
            return;
        }
        graceful_shutdown(handle);
    });
}

/// Stop taking work, let what is in flight finish, then release every device.
///
/// Extracted from the `RunEvent::Exit` closure so a **second** exit path can call the same
/// sequence. The updater is that second path, and it calls this directly — before handing
/// the bytes to the installer, not from an exit hook. Two reasons it cannot be a hook: the
/// plugin's `install` ends by calling `process::exit` itself, which never emits
/// `RunEvent::Exit`, and
/// its `on_before_exit` callback fires inside the async runtime where the `block_on` calls
/// below would panic. Skipping the sequence would leak a WDA relay, an XCTest runner and an
/// adb forward per phone, on every update.
///
/// Called from a plain OS thread there, for the same `block_on` reason.
///
/// Idempotent by construction: every step is either a flag set to the value it already has
/// or a shutdown that no-ops once done, so being called from both paths is safe.
pub(crate) fn graceful_shutdown(handle: &tauri::AppHandle) {
    let Some(state) = handle.try_state::<AppState>() else {
        return;
    };
    {
        state.reject_new_work();
        state.nurture.begin_shutdown();
        state.flows.stop_all();
        state.jobs.stop_all();
        tauri::async_runtime::block_on(state.wait_for_mutating_commands());
        tauri::async_runtime::block_on(state.close_all_overlay_sessions());
        tauri::async_runtime::block_on(state.shutdown_android_views());
        let control = state.control.clone();
        if let Err(error) = tauri::async_runtime::block_on(state.shutdown_background_sampler()) {
            log::error!("background sampler shutdown failed: {error:#}");
        }
        if let Err(error) = tauri::async_runtime::block_on(state.flows.shutdown()) {
            log::error!("Flow runtime shutdown failed: {error:#}");
        }
        if let Err(error) = tauri::async_runtime::block_on(state.jobs.shutdown()) {
            log::error!("job queue shutdown failed: {error:#}");
        }
        if let Err(error) = tauri::async_runtime::block_on(control.shutdown_cleanup()) {
            log::error!("device cleanup shutdown failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every `#[tauri::command]` in a file, as (name, body).
    fn commands_in(source: &str) -> Vec<(&str, &str)> {
        source
            .split("#[tauri::command]")
            .skip(1)
            .filter_map(|chunk| {
                let signature = chunk.find("fn ")? + 3;
                let tail = &chunk[signature..];
                let name = &tail[..tail.find(['(', '<'])?];
                let end = chunk.find("\n#[").unwrap_or(chunk.len());
                Some((name, &chunk[..end]))
            })
            .collect()
    }

    /// The local login is gone, and this is what keeps it gone.
    ///
    /// It looks for the word rather than for the four removed handler names, because naming
    /// them would only catch someone re-adding *those*. The old pair stored the password
    /// verbatim in a column called `password_hash` and compared it as plaintext, and the next
    /// attempt would not have to be called `auth_login` to repeat that.
    ///
    /// **One** command legitimately touches a password, and it is named here rather than
    /// skipped, because that is a decision someone made and can be re-read:
    ///
    /// - `set_apple_id` takes the Apple ID app-specific password needed to resign WDA. It
    ///   hands it to the OS credential store, never to `state.db`, and `get_apple_id` reads
    ///   back only `has_password` — asserted below.
    ///
    /// It used to be two. `export_proxy_config` printed a proxy password the operator had
    /// typed in, and the entry argued -- correctly -- that a proxy password cannot be hashed
    /// because it has to survive round-trip. What the entry did not say is that **no UI could
    /// reach any of it**: the proxy feature had been removed from the interface, so the app was
    /// keeping a table with a readable password column for a feature that did not exist. The
    /// whole slice went in migration 16, and this list got shorter as a result -- which is the
    /// better way for a security exemption to end than a better-argued paragraph.
    ///
    /// A second entry appearing here means a new password surface arrived without that decision
    /// being made. Before the login was removed this failed on `farm_commands.rs::auth_login`.
    #[test]
    fn no_command_stores_a_login_password() {
        let surfaces = [
            ("commands/mod.rs", include_str!("commands/mod.rs")),
            (
                "commands/android_ops.rs",
                include_str!("commands/android_ops.rs"),
            ),
            ("commands/device.rs", include_str!("commands/device.rs")),
            ("commands/jobs.rs", include_str!("commands/jobs.rs")),
            ("commands/system.rs", include_str!("commands/system.rs")),
            ("commands/view.rs", include_str!("commands/view.rs")),
            ("farm_commands.rs", include_str!("farm_commands.rs")),
            ("agent_commands.rs", include_str!("agent_commands.rs")),
            ("flow_commands.rs", include_str!("flow_commands.rs")),
            ("nurture_commands.rs", include_str!("nurture_commands.rs")),
            ("publish_commands.rs", include_str!("publish_commands.rs")),
            (
                "interaction_commands.rs",
                include_str!("interaction_commands.rs"),
            ),
        ];
        let mut holders = Vec::new();
        for (file, source) in surfaces {
            for (name, body) in commands_in(source) {
                if body.to_ascii_lowercase().contains("password") {
                    holders.push((file, name, body));
                }
            }
        }
        let mut named = holders
            .iter()
            .map(|(file, name, _)| format!("{file}::{name}"))
            .collect::<Vec<_>>();
        named.sort();
        assert_eq!(named, vec!["commands/system.rs::set_apple_id".to_string()],);
        let (_, _, apple) = holders
            .iter()
            .find(|(_, name, _)| *name == "set_apple_id")
            .expect("set_apple_id");
        assert!(
            apple.contains("state.signing") && !apple.contains("state.db"),
            "set_apple_id must hand the password to the credential store, not the database:\n{apple}"
        );
    }

    /// Every command source file. Adding one and forgetting it here is the failure mode the
    /// list below is written to make loud, so it is asserted against the directory listing.
    const COMMAND_SOURCES: &[(&str, &str)] = &[
        ("agent_commands.rs", include_str!("agent_commands.rs")),
        ("commands/mod.rs", include_str!("commands/mod.rs")),
        (
            "commands/android_ops.rs",
            include_str!("commands/android_ops.rs"),
        ),
        ("commands/device.rs", include_str!("commands/device.rs")),
        ("commands/jobs.rs", include_str!("commands/jobs.rs")),
        ("commands/system.rs", include_str!("commands/system.rs")),
        ("commands/view.rs", include_str!("commands/view.rs")),
        ("farm_commands.rs", include_str!("farm_commands.rs")),
        ("flow_commands.rs", include_str!("flow_commands.rs")),
        (
            "interaction_commands.rs",
            include_str!("interaction_commands.rs"),
        ),
        ("lib.rs", include_str!("lib.rs")),
        ("local_api.rs", include_str!("local_api.rs")),
        ("nurture_commands.rs", include_str!("nurture_commands.rs")),
        ("peripherals.rs", include_str!("peripherals.rs")),
        ("publish_commands.rs", include_str!("publish_commands.rs")),
    ];

    /// Commands that may skip `ensure_accepting_work()`, each with the reason it may.
    ///
    /// This is the **whole** exemption list: anything not named here must hold admission, so a
    /// new command is guarded by default and skipping it is a decision someone has to write
    /// down. That is the inversion — see the test below for why the previous shape could not
    /// work.
    const ADMISSION_EXEMPT: &[(&str, &str)] = &[
        // Reads. They answer from the DB, from memory, or from a frame already captured, and
        // touch no device — so refusing them during shutdown drain would blank the UI for no
        // safety gained.
        ("agent_get_settings", "read: agent settings held in memory"),
        (
            "agent_list_statuses",
            "read: cached_agent_status, probes nothing",
        ),
        ("list_devices", "read: cached roster"),
        ("get_stream_settings", "read: KV config"),
        ("latest_frame", "read: last frame already in memory"),
        ("view_endpoint", "read: the loopback URL"),
        ("view_report_paint", "read-back: frontend paint counters"),
        ("list_jobs", "read: DB"),
        ("list_scripts", "read: DB"),
        ("example_script", "read: a static fixture"),
        (
            "get_apple_id",
            "read: email + has_password, never the password",
        ),
        ("driver_mode", "read: which backend is live"),
        ("driver_degraded_reason", "read: health probe"),
        ("android_unavailable_reason", "read: health probe"),
        ("android_tool_problems", "read: boot snapshot"),
        (
            "log_frontend_error",
            "the frontend's only path to the log; refusing it during drain would silence              exactly the errors that explain the drain",
        ),
        ("update_check", "read: asks GitHub, touches no device"),
        ("get_device_meta", "read: DB"),
        ("list_device_metas", "read: DB"),
        ("list_groups", "read: DB"),
        ("list_materials", "read: DB"),
        ("list_apps_library", "read: DB"),
        ("list_schedules", "read: DB"),
        ("list_op_logs", "read: DB"),
        ("analytics_summary", "read: DB aggregate"),
        ("api_docs", "read: static text"),
        ("flow_action_catalog", "read: static catalog"),
        ("flow_list", "read: DB"),
        ("flow_get", "read: DB"),
        ("flow_validate", "pure: compiles a document, no I/O"),
        (
            "flow_import_legacy",
            "pure: parses JSON into a typed document",
        ),
        ("flow_export", "read: DB"),
        ("flow_list_runs", "read: DB"),
        ("flow_get_run", "read: DB"),
        ("flow_read_artifact", "read: DB-keyed bytes, hash-verified"),
        ("interaction_parse_links", "pure: string parsing"),
        (
            "interaction_resolve_links",
            "read: follows TikTok redirects, touches no device",
        ),
        (
            "interaction_preview_thread",
            "read: plans a campaign and reads the in-memory stream budget; touches no device",
        ),
        ("interaction_list", "read: DB"),
        ("interaction_get", "read: DB"),
        ("interaction_list_target_notes", "read: DB"),
        ("interaction_list_artifacts", "read: DB"),
        (
            "interaction_read_artifact",
            "read: DB-keyed bytes, hash-verified",
        ),
        ("local_api_get_config", "read: KV config"),
        ("nurture_get_settings", "read: DB"),
        ("nurture_list_comment_attempts", "read: DB"),
        ("nurture_cost_summary", "read: DB aggregate"),
        ("nurture_session_status", "read: in-memory session state"),
        ("nurture_session_log", "read: in-memory ring, one device"),
        (
            "nurture_session_log_summary",
            "read: in-memory ring, all devices",
        ),
        (
            "nurture_clear_session_log",
            "drops an in-memory ring the operator asked to clear; touches no device",
        ),
        ("publish_list", "read: DB"),
        ("publish_get", "read: DB"),
        // Not reads, and not oversights — each guards differently, on purpose.
        (
            "update_install",
            "guards with state.busy_reason(): admission would let it install mid-run",
        ),
        (
            "retry_startup",
            "re-runs bootstrap; there may be no admission gate yet to hold",
        ),
        ("startup_error", "read: why bootstrap failed"),
    ];

    /// Every `#[tauri::command]`, with its body, across every command file.
    fn all_commands() -> Vec<(&'static str, &'static str, &'static str)> {
        let mut found = Vec::new();
        for (file, source) in COMMAND_SOURCES {
            // Cut the trailing test *module* first: `lib.rs`'s own tests contain the literal
            // "#[tauri::command]" (this scanner splits on it), so counting those would make
            // the test read itself.
            //
            // Matched as `#[cfg(test)]` immediately followed by `mod `, not as a bare
            // `#[cfg(test)]`: several files carry item-level `#[cfg(test)]` on test-only
            // imports at the *top*, and cutting there truncated the entire file. Not
            // hypothetical — it hid all six commands in `agent_commands.rs`, and the
            // cross-check below is what caught it.
            //
            // Matched **newline-agnostically**, and that is not a detail. This repo is developed
            // on Windows with `core.autocrlf=true`: the index holds LF and a fresh checkout
            // writes CRLF, so a `"#[cfg(test)]\nmod "` needle finds nothing there. The cut then
            // never happens, the scan reads this file's own test module, and it reports the test
            // helpers `commands_in` and `all_commands` as commands answering in the wrong shape.
            // It passed here for months only because these files happened to have been written
            // by an editor rather than by `git checkout` — one history rewrite was enough to
            // turn it red, which means every clone on a Windows box was already red.
            let cut = source
                .match_indices("#[cfg(test)]")
                .find(|(at, marker)| {
                    source[at + marker.len()..]
                        .trim_start_matches(['\r', '\n'])
                        .starts_with("mod ")
                })
                .map(|(at, _)| at);
            let source = match cut {
                Some(at) => &source[..at],
                None => source,
            };
            for chunk in source.split("#[tauri::command]").skip(1) {
                let Some(at) = chunk.find("fn ") else {
                    continue;
                };
                let tail = &chunk[at + 3..];
                let Some(stop) = tail.find(['(', '<']) else {
                    continue;
                };
                found.push((*file, &tail[..stop], chunk));
            }
        }
        found
    }

    /// Commands that answer without a `Result`, and why each one cannot fail.
    ///
    /// Small and meant to stay small: an infallible command is one whose answer is already in
    /// memory. Anything that touches a device, the database or the filesystem belongs in the
    /// other list.
    const INFALLIBLE_COMMANDS: &[(&str, &str)] = &[
        ("agent_get_settings", "returns settings held in memory"),
        ("agent_list_statuses", "returns the cached status map"),
        ("android_tool_problems", "returns a stored Vec<String>"),
        (
            "android_unavailable_reason",
            "returns a stored Option<String>",
        ),
        ("api_docs", "returns a &'static str"),
        ("driver_degraded_reason", "returns a stored Option<String>"),
        ("driver_mode", "returns an enum the state already holds"),
        ("example_script", "returns a &'static str"),
        ("flow_action_catalog", "returns the compiled-in catalog"),
        (
            "get_stream_settings",
            "returns the in-memory copy of the KV row",
        ),
        ("interaction_parse_links", "pure text parsing, cannot fail"),
        (
            "log_frontend_error",
            "writes a log line and returns nothing; a reporting path that can fail has              nowhere to report the failure",
        ),
        ("startup_error", "returns a stored Option<String>"),
        ("view_report_paint", "records a paint and returns nothing"),
    ];

    /// Every fallible command answers the frontend in one shape.
    ///
    /// It did not. 57 of 163 returned `Result<_, String>`, and two whole modules --
    /// `farm_commands.rs` (25/26) and `publish_commands.rs` (8/8) -- were entirely String, so
    /// the boundary ran along module history rather than along anything a caller could reason
    /// about. A `String` error reaches the webview as a bare string: no `code`, so the
    /// frontend cannot tell "the app is shutting down" from "the phone is unplugged" except by
    /// matching on prose. Six commands even took a `CommandError` the admission gate had
    /// already built and flattened it with `.map_err(String::from)`, throwing away the
    /// `ApplicationShuttingDown` code on the way out.
    ///
    /// This is the test that turns adding a 164th command in the old shape into a build
    /// failure rather than a slow return to two conventions.
    #[test]
    fn every_command_answers_in_one_error_shape() {
        let mut wrong = Vec::new();
        let mut infallible_seen = Vec::new();

        for (file, name, chunk) in all_commands() {
            // The return type: from `->` to the `{` that opens the body, balanced on `<>` so
            // a nested generic cannot end the scan early. That imprecision is what made the
            // first count of this surface wrong by 14.
            let signature = &chunk[..chunk.find('{').unwrap_or(chunk.len())];
            let Some(arrow) = signature.find("->") else {
                infallible_seen.push(name);
                continue;
            };
            let mut depth = 0i32;
            let ret: String = signature[arrow + 2..]
                .chars()
                .take_while(|c| {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        _ => {}
                    }
                    true
                })
                .collect();
            let ret = ret.trim();

            if !ret.starts_with("Result") {
                infallible_seen.push(name);
            } else if ret.contains("CommandError") {
                // `Vec<CommandError>` is deliberate on the validation path: the frontend
                // renders every issue at once rather than the first one.
            } else {
                wrong.push(format!("{file}::{name} -> {ret}"));
            }
        }

        assert!(
            wrong.is_empty(),
            "these commands do not answer with CommandError:
  {}",
            wrong.join(
                "
  "
            )
        );

        // And the infallible list is a list, not a loophole: a command that quietly stops
        // returning a Result has to say so here.
        let declared: Vec<&str> = INFALLIBLE_COMMANDS.iter().map(|(n, _)| *n).collect();
        for name in &infallible_seen {
            assert!(
                declared.contains(name),
                "`{name}` returns no Result and is not in INFALLIBLE_COMMANDS with a reason"
            );
        }
        for (name, _) in INFALLIBLE_COMMANDS {
            assert!(
                infallible_seen.contains(name),
                "`{name}` is listed as infallible but now returns a Result"
            );
        }
    }

    /// The pinned toolchain and the one CI installs must be the same version.
    ///
    /// `rust-toolchain.toml` exists so a developer's `cargo clippy` runs the lints CI runs. If
    /// the two drift, it does the opposite of its job — it pins the machine to a version CI is
    /// *not* using, and the lint difference it was added to remove comes back silently. That
    /// difference is not hypothetical: clippy 1.97 flagged an `unnecessary_cast` at `fa8ecca`
    /// that the release toolchain did not.
    #[test]
    fn the_pinned_toolchain_matches_the_one_ci_installs() {
        let pinned = include_str!("../../../../rust-toolchain.toml");
        let workflow = include_str!("../../../../.github/workflows/desktop-ci-cd.yml");

        let channel = pinned
            .lines()
            .find_map(|line| line.trim().strip_prefix("channel = "))
            .map(|value| value.trim_matches('"'))
            .expect("rust-toolchain.toml declares a channel");
        let ci = workflow
            .lines()
            .find_map(|line| line.trim().strip_prefix("RUST_TOOLCHAIN: "))
            .map(|value| value.trim_matches('"'))
            .expect("the workflow declares RUST_TOOLCHAIN");

        assert_eq!(
            channel, ci,
            "rust-toolchain.toml pins {channel} but CI installs {ci}"
        );
    }

    /// Admission is required by **default**, and skipping it has to be written down.
    ///
    /// This replaces an allowlist of 84 command names that each had to hold
    /// `ensure_accepting_work()`. The problem with that shape is not that it was incomplete —
    /// it is that it **could not** be complete: a new mutating command that forgot admission
    /// *and* was not added to the list passed, so the gate could not catch the one mistake it
    /// exists to catch. Three whole files (`interaction_commands.rs`, `peripherals.rs`,
    /// `local_api.rs` — 16 commands) had never been in its inventory at all.
    ///
    /// Inverted, the default is safe: a command is checked unless someone names it and says
    /// why. Measured when this landed: 158 commands, 52 exempt, and no device-mutating command
    /// was missing admission — so this closes a drift path rather than a live hole.
    #[test]
    fn every_command_holds_admission_unless_explicitly_exempted() {
        let exempt: std::collections::HashMap<&str, &str> =
            ADMISSION_EXEMPT.iter().copied().collect();
        let mut offenders = Vec::new();
        for (file, name, body) in all_commands() {
            if body.contains("ensure_accepting_work()") || exempt.contains_key(name) {
                continue;
            }
            offenders.push(format!("{file}::{name}"));
        }
        assert!(
            offenders.is_empty(),
            "these commands neither hold ensure_accepting_work() nor appear in ADMISSION_EXEMPT \
             with a reason: {}",
            offenders.join(", ")
        );
    }

    /// The scan sees every command that is actually registered — proved, not assumed.
    ///
    /// `all_commands()` stops at each file's `#[cfg(test)]`, because `lib.rs`'s own test module
    /// contains the literal `"#[tauri::command]"` and splitting on it would make this test read
    /// itself. That cut is also a blind spot: a command written *below* the test module would
    /// be invisible to the gate, and a gate with an invisible region is the thing this whole
    /// inversion is trying to stop being. Found the honest way — an early probe of the gate was
    /// appended to the end of a file, the gate stayed green, and the probe rather than the gate
    /// turned out to be wrong.
    ///
    /// So: cross-check the scan against `generate_handler!`, which is the list Tauri actually
    /// exposes. A command hidden below a test module, or defined and never registered, makes
    /// the two disagree.
    #[test]
    fn the_admission_scan_sees_every_registered_command() {
        let source = include_str!("lib.rs");
        let at = source
            .find("generate_handler!")
            .expect("lib.rs registers its commands with generate_handler!");
        let tail = &source[at..];
        let open = tail.find('[').expect("generate_handler![ ... ]");
        let close = tail.find(']').expect("generate_handler![ ... ]");
        let registered: std::collections::HashSet<&str> = tail[open + 1..close]
            .split(',')
            .map(|entry| entry.trim())
            .filter(|entry| !entry.is_empty() && !entry.starts_with("//"))
            .map(|entry| entry.rsplit("::").next().unwrap_or(entry))
            .collect();

        let scanned: std::collections::HashSet<&str> = all_commands()
            .into_iter()
            .map(|(_, name, _)| name)
            .collect();

        let missed: Vec<&str> = registered.difference(&scanned).copied().collect();
        assert!(
            missed.is_empty(),
            "registered but invisible to the admission scan (below a #[cfg(test)] module?): {}",
            missed.join(", ")
        );
    }

    /// An exemption that no longer names a real command is a stale claim, so it fails too.
    ///
    /// Without this the list only ever grows: a command gets renamed or deleted, its excuse
    /// stays behind, and the next reader believes a decision was made about something that is
    /// not there.
    #[test]
    fn no_admission_exemption_outlives_its_command() {
        let commands: std::collections::HashSet<&str> = all_commands()
            .into_iter()
            .map(|(_, name, _)| name)
            .collect();
        let stale: Vec<&str> = ADMISSION_EXEMPT
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| !commands.contains(name))
            .collect();
        assert!(
            stale.is_empty(),
            "ADMISSION_EXEMPT names commands that no longer exist: {}",
            stale.join(", ")
        );

        // And an exemption that has since grown a real admission call is also stale — it now
        // claims an exception it does not take.
        let redundant: Vec<&str> = all_commands()
            .into_iter()
            .filter(|(_, name, body)| {
                body.contains("ensure_accepting_work()")
                    && ADMISSION_EXEMPT.iter().any(|(exempt, _)| exempt == name)
            })
            .map(|(_, name, _)| name)
            .collect();
        assert!(
            redundant.is_empty(),
            "these hold admission and no longer need an exemption: {}",
            redundant.join(", ")
        );
    }

    /// Commands in `android_ops.rs` that reach a phone **without** holding it, and why.
    ///
    /// Inverted like `ADMISSION_EXEMPT`, and for the same reason: the safe answer is the default,
    /// and skipping it costs somebody a written sentence. Before this list existed, all
    /// twenty-four commands in that file were in it implicitly and none of them said so.
    const LEASE_EXEMPT: &[(&str, &str)] = &[
        (
            "arp_scan",
            "no udid at all: reads this host's ARP table, touches no phone",
        ),
        (
            "is_rooted",
            "read-only: asks the phone `su -c id` and reports the answer",
        ),
        (
            "device_list_dir",
            "read-only: one `ls -la`, changes nothing",
        ),
        (
            "device_pull_path",
            "reads the phone and writes this host. Leasing it would block a running session \
             for up to the 300 s transfer timeout, which is worse than the interleaving it \
             would prevent; the adb transfer sub-cap is what bounds its cost",
        ),
        (
            "wifi_adb_connect",
            "takes a host:port rather than a udid, so there is no phone to lease yet",
        ),
        (
            "wifi_adb_disconnect",
            "same: a host:port, and the phone may already be gone",
        ),
    ];

    /// **Every Android-only command holds the phone, or names itself and says why not.**
    ///
    /// The hole this closes: `commands/android_ops.rs` held admission on all twenty-four of its
    /// commands and a **device lease on none of them**, so `factory_reset`, `root_shell`,
    /// `power_off_device`, `device_delete_path`, `set_input_method` and `open_system_settings`
    /// could all fire at a phone another piece of work was holding. The failure was not the
    /// action failing; it was the action *succeeding* while a nurture session went on tapping
    /// coordinates into whatever was now in front and **reported the work done**.
    ///
    /// Scoped to this one file on purpose. The same measurement across the rest is
    /// `commands/device.rs` 25 of 32, `farm_commands.rs` 3 of 26, `commands/view.rs` 0 of 10,
    /// `interaction_commands.rs` 4 of 13 — mostly reads that are fine, but fifty-odd written
    /// reasons is its own piece of work, and a half-filled allowlist is a gate that reads as
    /// stronger than it is.
    #[test]
    fn every_android_only_command_holds_the_phone_or_says_why_not() {
        let source = include_str!("commands/android_ops.rs");
        let exempt: std::collections::HashMap<&str, &str> = LEASE_EXEMPT.iter().copied().collect();
        let scanned = commands_in(source);

        let mut offenders = Vec::new();
        for (name, body) in &scanned {
            if body.contains("hold_this_phone(") || exempt.contains_key(name) {
                continue;
            }
            offenders.push(*name);
        }
        assert!(
            offenders.is_empty(),
            "these reach a phone without holding it and are not in LEASE_EXEMPT with a reason: \
             {}",
            offenders.join(", ")
        );

        // Anti-rot: a scan that silently matched nothing would make the assertion above pass
        // for the worst possible reason.
        assert!(
            scanned.len() >= 20,
            "the scan found only {} commands in android_ops.rs; it used to hold 24",
            scanned.len()
        );

        // And no exemption outlives its command, or the list slowly becomes a wish.
        let names: std::collections::HashSet<&str> =
            scanned.iter().map(|(name, _)| *name).collect();
        for (name, _) in LEASE_EXEMPT {
            assert!(
                names.contains(name),
                "LEASE_EXEMPT names `{name}`, which no longer exists in android_ops.rs"
            );
        }

        // An exemption that has since grown a lease is also stale, in the other direction.
        for (name, body) in &scanned {
            if exempt.contains_key(name) && body.contains("hold_this_phone(") {
                panic!("`{name}` now holds the phone; drop it from LEASE_EXEMPT");
            }
        }
    }

    #[test]
    fn exit_order_cancels_workers_then_drains_commands_before_cleanup() {
        // Scope the search to the app.run(...) exit closure. Searching the whole
        // file matched the `ordered` array literal below — which is, of course,
        // already in order — so the test passed regardless of the real sequence.
        //
        // Retargeted from the `app.run(...)` closure when the body moved out into
        // `graceful_shutdown`, so a second exit path — the updater, which quits the app to
        // hand over to the installer — could call the same sequence.
        // `every_exit_path_runs_the_graceful_shutdown` is the other half: this test checks
        // the order, that one checks nothing exits around it.
        let source = include_str!("lib.rs");
        let run_start = source
            .find("fn graceful_shutdown(")
            .expect("graceful_shutdown present");
        let run_end = source[run_start..]
            .find("\n#[cfg(test)]")
            .map(|index| run_start + index)
            .unwrap_or(source.len());
        let exit_flow = &source[run_start..run_end];
        let ordered = [
            "state.reject_new_work()",
            "state.nurture.begin_shutdown()",
            "state.flows.stop_all()",
            "state.jobs.stop_all()",
            "state.wait_for_mutating_commands()",
            "state.close_all_overlay_sessions()",
            "state.shutdown_android_views()",
            "state.shutdown_background_sampler()",
            "state.flows.shutdown()",
            "state.jobs.shutdown()",
            "control.shutdown_cleanup()",
        ];
        let mut offset = 0;
        for operation in ordered {
            let position = exit_flow[offset..]
                .find(operation)
                .unwrap_or_else(|| panic!("missing shutdown operation {operation}"));
            offset += position + operation.len();
        }
    }

    #[test]
    fn the_updater_releases_the_fleet_between_downloading_and_installing() {
        // Three steps, and the order carries all the safety:
        //   busy check first — the press happens however long after the check the operator
        //                      took to read it, and a session can start in between
        //   download next    — a failed download must leave the fleet exactly as it was
        //   shutdown then    — `install` ends in `process::exit(0)` on Windows, so the
        //                      normal exit path never runs and every relay would leak
        //   install last     — nothing irreversible until the phones are already released
        //
        // Re-ordering any pair still compiles and still works on a farm with no phones
        // plugged in, which is exactly why this is pinned rather than left to review.
        // **Normalised first.** The needle below is `\n}\n`, and `rustc` folds CRLF to LF inside
        // string literals whatever this file's own line endings are — while `include_str!`
        // returns the scanned file's bytes as they sit on disk. On a checkout with
        // `core.autocrlf=true` the file carries CR, the needle does not, and the search
        // fails with *update_install has a body* on a tree where the body is right there.
        //
        // CI cannot catch this: the workflow checks out with `core.autocrlf false`. Measured
        // 26/08/2026 on a clone where a merge had just rewritten the tree.
        let source = &include_str!("commands/system.rs").replace("\r\n", "\n");
        let start = source
            .find("pub async fn update_install(")
            .expect("update_install present");
        let body = &source[start..];
        // Column-zero brace: the function's own close, since every inner block is indented.
        let end = body.find("\n}\n").expect("update_install has a body");
        let body = &body[..end];
        let ordered = [
            "state.busy_reason()",
            ".download(",
            "graceful_shutdown(&handle)",
            ".install(bytes)",
        ];
        let mut offset = 0;
        for step in ordered {
            let position = body[offset..]
                .find(step)
                .unwrap_or_else(|| panic!("update_install is missing or re-orders {step}"));
            offset += position + step.len();
        }
    }

    #[test]
    fn every_exit_path_runs_the_graceful_shutdown() {
        // The leak this guards is silent and fleet-wide: an exit that skips the sequence
        // leaves WDA relays, XCTest runners and adb forwards behind on every phone, and
        // nothing reports it. `process::exit` is exactly such an exit, and it is what an
        // updater does by default — so the rule is that any exit path routes through
        // `graceful_shutdown`, and this asserts the routing instead of trusting it.
        let source = include_str!("lib.rs");
        assert!(
            source.contains("graceful_shutdown(handle)"),
            "no exit path calls graceful_shutdown; the whole sequence would be dead code"
        );

        // Nothing may bypass it by killing the process directly. Checked over the file
        // outside this test module, so the assertion cannot match its own text.
        let end = source
            .find("\n#[cfg(test)]")
            .expect("the test module marks the end of the production body");
        let production = &source[..end];
        // The open paren matters: prose about `process::exit` — including the doc comment
        // on `graceful_shutdown` explaining why it is forbidden — must not trip this. Only
        // a call site has it.
        for forbidden in ["process::exit(", "process::abort("] {
            assert!(
                !production.contains(forbidden),
                "{forbidden} bypasses graceful_shutdown; route that exit through it instead"
            );
        }
    }

    // Named rather than glob-imported: this module has no `use super::*` on purpose, because
    // its own scans read the file as text and a wildcard would quietly widen what they see.
    use super::{panic_message, panic_report};

    /// A panic line has to answer three questions, or it is not worth the abort it records.
    #[test]
    fn a_panic_report_names_the_message_the_place_and_the_thread() {
        let line = panic_report(
            "context activity count cannot underflow",
            Some("crates/core/src/device_control/mod.rs:930:9"),
            "tokio-runtime-worker",
        );
        assert!(line.contains("context activity count cannot underflow"));
        assert!(line.contains("crates/core/src/device_control/mod.rs:930:9"));
        assert!(line.contains("tokio-runtime-worker"));
    }

    /// **A missing location must not silently shorten the line.**
    ///
    /// `PanicHookInfo::location()` is an `Option`, and the tempting shape is
    /// `if let Some(location)` around the whole report -- which drops the record entirely in
    /// the one case where the reader has least to go on.
    #[test]
    fn a_panic_with_no_location_still_reports_the_message() {
        let line = panic_report("the cleanup worker channel closed", None, "unnamed");
        assert!(line.contains("the cleanup worker channel closed"));
        assert!(line.contains("unnamed"));
        assert!(
            line.contains("unknown location"),
            "say the location is unknown rather than leaving a gap: {line}"
        );
    }

    /// All three payload shapes have to arrive as readable text.
    ///
    /// `panic!("literal")` is `&'static str`, `panic!("{}", x)` is `String`, and `panic_any`
    /// can carry a type neither branch knows. The third is why there is a fallback string at
    /// all: without it the log line would name a thread and a file and say nothing happened.
    #[test]
    fn every_panic_payload_shape_reaches_the_log() {
        let as_str: Box<dyn std::any::Any + Send> = Box::new("a &str payload");
        assert_eq!(panic_message(as_str.as_ref()), "a &str payload");

        let as_string: Box<dyn std::any::Any + Send> = Box::new(String::from("a String payload"));
        assert_eq!(panic_message(as_string.as_ref()), "a String payload");

        let as_other: Box<dyn std::any::Any + Send> = Box::new(42_u32);
        let described = panic_message(as_other.as_ref());
        assert!(
            !described.is_empty() && described.contains("neither"),
            "an odd payload must still describe itself, got {described:?}"
        );
    }

    /// **The hook has to be installed before the first thing that can panic, and the first
    /// thing that can panic is on the very next line.**
    ///
    /// `run()` opens with `install_process_tree_guard().expect(...)`. A hook registered after
    /// it would leave the one startup failure an operator actually hits -- another copy of the
    /// app already owning the process tree -- as the silent vanishing window this whole change
    /// is about. Ordering, not presence, is the property; so the test reads the order.
    #[test]
    fn the_panic_hook_is_installed_before_the_first_thing_that_can_panic() {
        let source = include_str!("lib.rs");
        let at = source
            .find("pub fn run() {")
            .expect("lib.rs defines the run() entrypoint");
        let body = &source[at..];
        let hook = body
            .find("install_panic_logging();")
            .expect("run() installs the panic hook");
        let guard = body
            .find("install_process_tree_guard()")
            .expect("run() claims the process tree");
        assert!(
            hook < guard,
            "the panic hook is installed after the first expect() in run(), so that expect()              still dies silently"
        );
    }

    /// The hook is load-bearing *because* of the release profile, so pin the reason.
    ///
    /// With `panic = "abort"` a panic in any spawned task ends the process rather than becoming
    /// a `JoinError`, and `strip = "symbols"` removes the backtrace that would otherwise let
    /// someone reconstruct it afterwards. If either ever changes, the reasoning in
    /// `install_panic_logging` needs re-reading rather than inheriting.
    #[test]
    fn the_release_profile_still_makes_a_panic_fatal_and_unsymbolised() {
        let manifest = include_str!("../../../../Cargo.toml");
        let profile = manifest
            .split("[profile.release]")
            .nth(1)
            .expect("the workspace manifest carries a release profile");
        assert!(
            profile.contains("panic = \"abort\""),
            "panic = abort is why a task panic kills the app; re-read install_panic_logging              if this changed"
        );
        assert!(
            profile.contains("strip = \"symbols\""),
            "strip = symbols is why the hook must say enough on its own"
        );
    }
}
