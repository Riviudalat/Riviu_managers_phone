use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::report;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_INPUT: i32 = 2;
pub const EXIT_REDACTION: i32 = 3;
pub const EXIT_GATE_BLOCKED: i32 = 4;

#[derive(Debug, Parser)]
#[command(name = "rtmmo-re", version, about = "RT-MMO forensic inventory")]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Inventory {
        #[arg(long)]
        ipa: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    BaselineVerify {
        #[arg(long)]
        lock: PathBuf,
        #[arg(long)]
        archive: PathBuf,
    },
    BaselineDiff {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        lock: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    GateA {
        #[arg(long)]
        ipa: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        routes: PathBuf,
        #[arg(long)]
        baseline_source: PathBuf,
        #[arg(long)]
        baseline_archive: PathBuf,
        #[arg(long)]
        baseline_lock: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    VerifyRedaction {
        #[arg(long, required = true)]
        input: Vec<PathBuf>,
    },
}

pub fn run(cli: Cli) -> i32 {
    let Some(command) = cli.command else {
        println!("rtmmo-re {}", env!("CARGO_PKG_VERSION"));
        return EXIT_SUCCESS;
    };

    match command {
        Command::Inventory { ipa, output } => input_result(|| {
            let inventory = report::inventory(&ipa)?;
            report::write_json(&output, &inventory)
        }),
        Command::VerifyRedaction { input } => match report::verify_redaction(&input) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                EXIT_REDACTION
            }
        },
        Command::BaselineVerify { lock, archive } => {
            input_result(|| report::verify_baseline(&lock, &archive))
        }
        Command::BaselineDiff {
            inventory,
            source,
            archive,
            lock,
            output,
        } => input_result(|| {
            let diff = report::baseline_diff(&inventory, &source, &lock, &archive)?;
            report::write_json(&output, &diff)
        }),
        Command::GateA {
            ipa,
            inventory,
            baseline,
            routes,
            baseline_source,
            baseline_archive,
            baseline_lock,
            manifest,
            output,
        } => match report::gate_a(report::GateAInputs {
            ipa: &ipa,
            inventory: &inventory,
            baseline: &baseline,
            routes: &routes,
            manifest: &manifest,
            baseline_source: &baseline_source,
            baseline_archive: &baseline_archive,
            baseline_lock: &baseline_lock,
        }) {
            Ok((passed, markdown)) => match report::write_text(&output, &markdown) {
                Ok(()) if passed => EXIT_SUCCESS,
                Ok(()) => EXIT_GATE_BLOCKED,
                Err(error) => {
                    eprintln!("{error:#}");
                    EXIT_INPUT
                }
            },
            Err(error) => {
                eprintln!("{error:#}");
                EXIT_INPUT
            }
        },
    }
}

fn input_result(action: impl FnOnce() -> anyhow::Result<()>) -> i32 {
    match action() {
        Ok(()) => EXIT_SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            EXIT_INPUT
        }
    }
}
