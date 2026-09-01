// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    match app_lib::deployment_check::parse_deployment_smoke_args(std::env::args_os()) {
        Ok(Some(args)) => {
            if let Err(error) = app_lib::run_deployment_smoke(args) {
                eprintln!("deployment startup smoke failed: {error:#}");
                std::process::exit(3);
            }
            return;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("invalid deployment smoke arguments: {error:#}");
            std::process::exit(3);
        }
    }
    app_lib::run();
}
