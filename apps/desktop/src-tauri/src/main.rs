// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.get(1).is_some_and(|value| value == "--verify-frontend") {
        let result = arguments.get(2)
            .filter(|_| arguments.len() == 3)
            .ok_or_else(|| anyhow::anyhow!("--verify-frontend requires one absolute report path"))
            .and_then(|path| app_lib::packaged_frontend::write_report(std::path::Path::new(path)));
        if let Err(error) = result {
            eprintln!("packaged frontend verification failed: {error:#}");
            std::process::exit(3);
        }
        return;
    }
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
