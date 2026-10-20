mod changes;
mod graph;
mod prove;
mod repo;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "neomemctl",
    about = "Control an isolated neomem and prove its features"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Prove(ProveArgs),
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct ProveArgs {
    feature: Option<String>,
    #[arg(long)]
    changed: bool,
    #[arg(long)]
    all: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Command::Prove(args) => prove::run(&selection_from(args)),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("neomemctl: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn selection_from(args: ProveArgs) -> prove::Selection {
    match (args.feature, args.changed) {
        (Some(feature), _) => prove::Selection::One(feature),
        (None, true) => prove::Selection::Changed,
        (None, false) => prove::Selection::All,
    }
}
