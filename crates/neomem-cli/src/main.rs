mod args;
mod connection;
mod hook;
mod intake_record;
mod login;
mod recall;
mod render;
mod time_argument;

use std::process::ExitCode;

use clap::Parser;
use std::path::PathBuf;

use neomem_core::Source;
use neomem_proto::{Envelope, Home, Request, Response, spool};

use crate::args::{Cli, Command, ModelsAction};
use crate::connection::DaemonConnection;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command_at = usize::from(arguments.first().is_some_and(|argument| argument == "--json"));
    if arguments.get(command_at).is_some_and(|argument| argument == "hook") {
        hook::run(&arguments[command_at + 1..]);
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    match run(&cli) {
        Ok(Outcome::Answered(response)) => render::print(&response, cli.json),
        Ok(Outcome::Spooled(path)) => render::print_spooled(&path, cli.json),
        Err(error) => {
            eprintln!("neomem: {error:#}");
            ExitCode::FAILURE
        }
    }
}

enum Outcome {
    Answered(Response),
    Spooled(PathBuf),
}

fn run(cli: &Cli) -> anyhow::Result<Outcome> {
    let home = Home::from_env().ok_or_else(|| anyhow::anyhow!("neither NEOMEM_HOME nor HOME is set"))?;
    let request = match &cli.command {
        Command::Recall(arguments) => return recall::run(&home, arguments).map(Outcome::Answered),
        Command::Hook { .. } => return Ok(Outcome::Answered(Response::Captured)),
        Command::Tags { new } => Request::Tags { new: new.clone() },
        Command::Link { external, to, kind } => Request::Link {
            external: external.parse()?,
            to: to.parse()?,
            kind: kind.parse().map_err(anyhow::Error::msg)?,
        },
        Command::Status => Request::Status,
        Command::Add(arguments) => Request::Add(intake_record::from_arguments(arguments)?),
        Command::Raw(arguments) => Request::Raw(intake_record::raw_query(arguments)?),
        Command::Resolve { name } => Request::Resolve { name: name.clone() },
        Command::Entity { name } => Request::Entity { name: name.clone() },
        Command::Show { id } => Request::Show { id: id.parse()? },
        Command::History { id } => Request::History { id: id.parse()? },
        Command::Useful { id } => Request::Useful { id: id.parse()? },
        Command::Rebuild => Request::Rebuild,
        Command::Dream => Request::Dream,
        Command::Reextract { source, from } => Request::Reextract {
            source: source.as_deref().map(neomem_core::Source::new).transpose()?,
            from: from.as_deref().map(time_argument::parse).transpose()?,
        },
        Command::Search(arguments) => Request::Search(intake_record::search_query(arguments)?),
        Command::Correct(arguments) => {
            Request::Correct { intake: arguments.intake.parse()?, reason: arguments.reason.clone() }
        }
        Command::Models(arguments) => match &arguments.action {
            None => Request::Models,
            Some(ModelsAction::Use { role, model }) => {
                Request::ModelsUse { role: role.model_role(), model: model.clone() }
            }
            Some(ModelsAction::Test { role, source, structured, inputs }) => Request::ModelsTest {
                role: role.model_role(),
                source: source.as_deref().map(Source::new).transpose()?,
                structured: *structured,
                inputs: inputs.clone(),
            },
        },
        Command::Login(arguments) => return login::run(&home, arguments).map(Outcome::Answered),
    };
    if request.waits_in_spool_when_the_daemon_is_down() {
        return match DaemonConnection::existing(&home) {
            Ok(connection) => connection.call(request).map(Outcome::Answered),
            Err(_) => Ok(Outcome::Spooled(spool::spool(&home, &Envelope::current(request))?)),
        };
    }
    DaemonConnection::open(&home)?.call(request).map(Outcome::Answered)
}
