mod agent_commands;
/// Public so the live harness binaries resolve the agent exactly as the app
/// does. Duplicating the token/manifest choice is how the two drift apart.
pub mod agent_runtime;
mod android_tools;
mod command_error;
mod commands;
mod farm_commands;
mod flow_commands;
mod interaction_commands;
pub mod interaction_ocr;
mod interaction_target;
mod local_api;
mod nurture_commands;
mod peripherals;
mod publish_commands;
mod publish_driver;
mod state;
mod view_hub;
mod view_watchdog;

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
) -> Result<Option<String>, String> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            commands::update_check,
            commands::update_install,
            farm_commands::get_device_meta,
            farm_commands::list_device_metas,
            farm_commands::save_device_meta,
            farm_commands::list_groups,
            farm_commands::save_group,
            farm_commands::delete_group,
            farm_commands::list_proxies,
            farm_commands::save_proxy,
            farm_commands::delete_proxy,
            farm_commands::export_proxy_config,
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
            farm_commands::list_publish_tasks,
            farm_commands::create_publish_task,
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
            interaction_commands::interaction_start_thread,
            interaction_commands::interaction_list,
            interaction_commands::interaction_get,
            interaction_commands::interaction_cancel,
            interaction_commands::interaction_retry,
            interaction_commands::interaction_open_on_device,
            interaction_commands::interaction_list_artifacts,
            interaction_commands::interaction_read_artifact,
            nurture_commands::nurture_get_settings,
            nurture_commands::nurture_save_settings,
            nurture_commands::nurture_test_api,
            nurture_commands::nurture_list_costs,
            nurture_commands::nurture_list_comment_attempts,
            nurture_commands::nurture_cost_summary,
            nurture_commands::nurture_session_status,
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
    /// Two commands legitimately touch a password and are named here rather than skipped,
    /// because each one is a decision someone made and can be re-read:
    ///
    /// - `set_apple_id` takes the Apple ID app-specific password needed to resign WDA. It
    ///   hands it to the OS credential store, never to `state.db`, and `get_apple_id` reads
    ///   back only `has_password` — asserted below.
    /// - `export_proxy_config` prints a proxy password the operator typed in. A proxy
    ///   password has to survive round-trip to be usable at all, so it cannot be hashed;
    ///   it is stored readable in `proxies` by design, not by the oversight this test is
    ///   about.
    ///
    /// A third entry appearing here means a new password surface arrived without that
    /// decision being made. Before the removal this failed on `farm_commands.rs::auth_login`.
    #[test]
    fn no_command_stores_a_login_password() {
        let surfaces = [
            ("commands.rs", include_str!("commands.rs")),
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
        assert_eq!(
            named,
            vec![
                "commands.rs::set_apple_id".to_string(),
                "farm_commands.rs::export_proxy_config".to_string(),
            ],
        );
        let (_, _, apple) = holders
            .iter()
            .find(|(_, name, _)| *name == "set_apple_id")
            .expect("set_apple_id");
        assert!(
            apple.contains("state.signing") && !apple.contains("state.db"),
            "set_apple_id must hand the password to the credential store, not the database:\n{apple}"
        );
    }

    fn command_body<'a>(source: &'a str, name: &str) -> &'a str {
        let sync = format!("pub fn {name}");
        let asynchronous = format!("pub async fn {name}");
        let start = source
            .find(&asynchronous)
            .or_else(|| source.find(&sync))
            .unwrap_or_else(|| panic!("missing mutating command {name}"));
        let tail = &source[start..];
        let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
        &tail[..end]
    }

    #[test]
    fn every_mutating_command_holds_application_admission() {
        let inventories = [
            (
                include_str!("commands.rs"),
                &[
                    "refresh_devices",
                    "prepare_device",
                    "install_ipa",
                    "install_ipa_to_group",
                    "install_unsigned_ipa",
                    "uninstall_app",
                    "list_installed_apps",
                    "device_shell",
                    "import_media",
                    "export_media",
                    "set_screen_rotation",
                    "screenshot",
                    "syslog",
                    "reboot_device",
                    "backup_device",
                    "restore_device",
                    "device_tap",
                    "device_swipe",
                    "device_swipe_path",
                    "device_type_text",
                    "device_home",
                    "device_key",
                    "device_control_begin",
                    "device_control_end",
                    "group_input",
                    "set_stream_settings",
                    "view_ensure",
                    "view_request_keyframe",
                    "view_inject_touch",
                    "view_set_preset",
                    "save_view_snapshot",
                    "run_script",
                    "cancel_job",
                    "save_script",
                    "set_apple_id",
                    "clear_apple_id",
                    "resign_wda",
                    "bulk_resign_wda",
                ][..],
            ),
            (
                include_str!("agent_commands.rs"),
                &[
                    "agent_save_settings",
                    "agent_preflight",
                    "agent_repair",
                    "agent_bulk_repair",
                ][..],
            ),
            (
                include_str!("farm_commands.rs"),
                &[
                    "save_device_meta",
                    "save_group",
                    "delete_group",
                    "save_proxy",
                    "delete_proxy",
                    "add_material",
                    "delete_material",
                    "push_material",
                    "add_app_library",
                    "delete_app_library",
                    "install_library_app",
                    "save_schedule",
                    "delete_schedule",
                    "create_publish_task",
                ][..],
            ),
            (
                include_str!("nurture_commands.rs"),
                &[
                    "nurture_save_settings",
                    "nurture_test_api",
                    "nurture_start",
                    "nurture_stop",
                ][..],
            ),
            (
                include_str!("flow_commands.rs"),
                &[
                    "flow_save_revision",
                    "flow_archive",
                    "flow_run",
                    "flow_cancel_run",
                    "flow_retry_attempt",
                    "flow_coordinate_frame",
                ][..],
            ),
            (
                include_str!("publish_commands.rs"),
                &[
                    "publish_create_campaign",
                    "publish_cancel",
                    "publish_prepare",
                    "publish_transfer",
                    "publish_post",
                ][..],
            ),
        ];

        for (source, commands) in inventories {
            for command in commands {
                assert!(
                    command_body(source, command).contains("ensure_accepting_work()"),
                    "mutating command {command} bypasses application admission"
                );
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
        let source = include_str!("commands.rs");
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
}
