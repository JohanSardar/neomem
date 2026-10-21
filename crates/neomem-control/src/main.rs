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
    Set {
        setting: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    Logs(HomeArgs),
    Prove(ProveArgs),
    Map(MapArgs),
    Bench(BenchArgs),
    Trace {
        #[arg(value_parser = ["search"])]
        what: String,
        text: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    FetchModels,
    Hook {
        agent: String,
        event: String,
        #[arg(long)]
        payload: Option<PathBuf>,
    },
    Replay {
        transcript: PathBuf,
        #[arg(long)]
        agent: Option<String>,
    },
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
    #[arg(value_parser = ["latency", "models"], default_value = "latency")]
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
        Command::Set { setting, home } => session::set(&setting, home.as_deref()),
        Command::Logs(args) => session::logs(args.home.as_deref()),
        Command::Prove(args) => prove::run(&selection_from(args)),
        Command::Map(args) => map::run(map_view(&args)),
        Command::Trace { text, home, .. } => session::trace_search(&text, home.as_deref()),
        Command::Bench(args) if args.kind == "models" => bench::models(),
        Command::Bench(args) => bench::run(args.save_baseline),
        Command::Hook { agent, event, payload } => {
            neomem_control::capture::hook(&agent, &event, payload.as_deref())
        }
        Command::Replay { transcript, agent } => {
            neomem_control::capture::replay(&transcript, agent.as_deref())
        }
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
