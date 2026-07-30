mod agent_commands;
mod agent_runtime;
mod command_error;
mod commands;
mod farm_commands;
mod nurture_commands;
mod state;

use state::AppState;
use tauri::{Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    riviu_ios_driver::install_process_tree_guard()
        .expect("failed to establish process-tree ownership");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let handle = app.handle().clone();
            let resource_dir = app.path().resource_dir().ok();
            let state = tauri::async_runtime::block_on(AppState::bootstrap(resource_dir))
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            state.spawn_background_tasks(handle.clone());
            handle.manage(state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            nurture_commands::nurture_get_settings,
            nurture_commands::nurture_save_settings,
            nurture_commands::nurture_list_costs,
            nurture_commands::nurture_cost_summary,
            nurture_commands::nurture_session_status,
            nurture_commands::nurture_start,
            nurture_commands::nurture_stop,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        if !matches!(event, RunEvent::Exit) {
            return;
        }
        let state = handle.state::<AppState>();
        state.nurture.begin_shutdown();
        state.jobs.stop_all();
        let control = state.control.clone();
        if let Err(error) = tauri::async_runtime::block_on(state.shutdown_background_sampler()) {
            log::error!("background sampler shutdown failed: {error:#}");
        }
        if let Err(error) = tauri::async_runtime::block_on(state.jobs.shutdown()) {
            log::error!("job queue shutdown failed: {error:#}");
        }
        if let Err(error) = tauri::async_runtime::block_on(control.shutdown_cleanup()) {
            log::error!("device cleanup shutdown failed: {error}");
        }
    });
}
