use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use neomem_control::isolated::Models;
use neomem_control::{bench, fake_models, map, model_files, prove, session};

#[derive(Debug, Parser)]
#[command(name = "neomemctl", about = "Control an isolated neomem and prove its features")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Up(UpArgs),
    Down(HomeArgs),
    Logs(HomeArgs),
    Prove(ProveArgs),
    Map(MapArgs),
    Bench(BenchArgs),
    FetchModels,
    #[command(hide = true)]
    FakeModels {
        #[arg(long)]
        address_file: PathBuf,
    },
}

#[derive(Debug, Args)]
struct UpArgs {
    #[arg(long, value_enum, default_value_t = Models::Fake)]
    models: Models,
    #[arg(long)]
    home: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BenchArgs {
    #[arg(value_parser = ["latency"], default_value = "latency")]
    kind: String,
    #[arg(long)]
    save_baseline: bool,
}

#[derive(Debug, Args)]
struct HomeArgs {
    #[arg(long)]
    home: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[group(multiple = false)]
struct MapArgs {
    #[arg(long)]
    next: bool,
    #[arg(long)]
    status: bool,
    #[arg(long)]
    check: bool,
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
        Command::Up(args) => session::up(args.models, args.home.as_deref()),
        Command::Down(args) => session::down(args.home.as_deref()),
        Command::Logs(args) => session::logs(args.home.as_deref()),
        Command::Prove(args) => prove::run(&selection_from(args)),
        Command::Map(args) => map::run(map_view(&args)),
        Command::Bench(args) => bench::run(args.save_baseline),
        Command::FetchModels => model_files::fetch_all(),
        Command::FakeModels { address_file } => fake_models::serve(&address_file),
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

fn map_view(args: &MapArgs) -> map::View {
    if args.next {
        map::View::Next
    } else if args.check {
        map::View::Check
    } else {
        map::View::Status
    }
}
