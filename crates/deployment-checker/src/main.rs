// Reuse the exact deployed checks without linking the Tauri desktop application.
#[path = "../../../apps/desktop/src-tauri/src/android_tools.rs"]
#[allow(dead_code)]
mod android_tools;
#[path = "../../../apps/desktop/src-tauri/src/deployment_check.rs"]
#[allow(dead_code)]
mod deployment_check;

fn main() {
    let args = match deployment_check::parse_args(std::env::args_os()) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error:#}\n{}", deployment_check::usage());
            std::process::exit(3);
        }
    };
    let report = match deployment_check::run(&args) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("deployment check failed internally: {error:#}");
            std::process::exit(3);
        }
    };
    if let Err(error) = deployment_check::write_report(&args.report, &report) {
        eprintln!("deployment report could not be written: {error:#}");
        std::process::exit(3);
    }
    let code = deployment_check::exit_code_for(&report, args.profile);
    println!(
        "{}",
        serde_json::json!({"ok":code==0,"overall":report.overall,"report":args.report})
    );
    std::process::exit(code);
}
