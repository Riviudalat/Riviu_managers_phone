use clap::Parser;

fn main() {
    let cli = rtmmo_re::cli::Cli::parse();
    std::process::exit(rtmmo_re::cli::run(cli));
}
