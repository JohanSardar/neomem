mod args;
mod connection;
mod intake_record;
mod login;
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
        Command::Status => Request::Status,
        Command::Add(arguments) => Request::Add(intake_record::from_arguments(arguments)?),
        Command::Raw(arguments) => Request::Raw(intake_record::raw_query(arguments)?),
        Command::Resolve { name } => Request::Resolve { name: name.clone() },
        Command::Entity { name } => Request::Entity { name: name.clone() },
        Command::Show { id } => Request::Show { id: id.parse()? },
        Command::Rebuild => Request::Rebuild,
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
