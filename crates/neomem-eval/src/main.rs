mod daemon;
mod data;
mod ingest;
mod model;
mod report;
mod runner;
mod score;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run(runner::Options),
    Report { directory: PathBuf },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Run(options) => runner::run(&options),
        Command::Report { directory } => report::write(&directory),
    }
}
