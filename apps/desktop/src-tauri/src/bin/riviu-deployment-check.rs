use app_lib::deployment_check::{exit_code_for, parse_args, run, usage, write_report};

fn main() {
    let args = match parse_args(std::env::args_os()) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error:#}\n{}", usage());
            std::process::exit(3);
        }
    };
    let report = match run(&args) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("deployment check failed internally: {error:#}");
            std::process::exit(3);
        }
    };
    if let Err(error) = write_report(&args.report, &report) {
        eprintln!("deployment report could not be written: {error:#}");
        std::process::exit(3);
    }
    let code = exit_code_for(&report, args.profile);
    println!(
        "{}",
        serde_json::json!({
            "ok": code == 0,
            "overall": report.overall,
            "report": args.report,
        })
    );
    std::process::exit(code);
}
