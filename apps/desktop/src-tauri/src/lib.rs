mod agent_commands;
mod agent_runtime;
mod command_error;
mod commands;
mod farm_commands;
mod flow_commands;
mod interaction_commands;
pub mod interaction_ocr;
mod nurture_commands;
mod publish_commands;
mod state;

use state::AppState;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

#[derive(Clone, Default)]
struct StartupState {
    error: Option<String>,
}

#[tauri::command]
fn startup_error(state: tauri::State<'_, StartupState>) -> Option<String> {
    state.error.clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    riviu_ios_driver::install_process_tree_guard()
        .expect("failed to establish process-tree ownership");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let window = if let Some(window) = app.get_webview_window("main") {
                window
            } else {
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Riviumanagersphone")
                    .inner_size(1440.0, 900.0)
                    .min_inner_size(1100.0, 700.0)
                    .resizable(true)
                    .visible(true)
                    .build()?
            };
            window.show()?;
            window.set_focus()?;
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

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
                            error: Some(message),
                        }
                    }
                };

            handle.manage(startup_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup_error,
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
            commands::uninstall_app,
            commands::screenshot,
            commands::syslog,
            commands::reboot_device,
            commands::device_tap,
            commands::device_swipe,
            commands::device_type_text,
            commands::device_home,
            commands::group_input,
            commands::get_stream_settings,
            commands::set_stream_settings,
            commands::latest_frame,
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
            farm_commands::auth_session,
            farm_commands::auth_login,
            farm_commands::auth_register,
            farm_commands::get_device_meta,
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
            farm_commands::list_users,
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
        if !matches!(event, RunEvent::Exit) {
            return;
        }
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        state.reject_new_work();
        state.nurture.begin_shutdown();
        state.flows.stop_all();
        state.jobs.stop_all();
        tauri::async_runtime::block_on(state.wait_for_mutating_commands());
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
    });
}

#[cfg(test)]
mod tests {
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
                    "uninstall_app",
                    "screenshot",
                    "syslog",
                    "reboot_device",
                    "device_tap",
                    "device_swipe",
                    "device_type_text",
                    "device_home",
                    "group_input",
                    "set_stream_settings",
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
                    "auth_session",
                    "auth_login",
                    "auth_register",
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
        let source = include_str!("lib.rs");
        let ordered = [
            "state.reject_new_work()",
            "state.nurture.begin_shutdown()",
            "state.flows.stop_all()",
            "state.jobs.stop_all()",
            "state.wait_for_mutating_commands()",
            "state.shutdown_background_sampler()",
            "state.flows.shutdown()",
            "state.jobs.shutdown()",
            "state.control.shutdown_cleanup()",
        ];
        let mut offset = 0;
        for operation in ordered {
            let position = source[offset..]
                .find(operation)
                .unwrap_or_else(|| panic!("missing shutdown operation {operation}"));
            offset += position + operation.len();
        }
    }
}
