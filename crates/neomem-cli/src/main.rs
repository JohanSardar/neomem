mod args;
mod connection;
mod intake_record;
mod login;
mod render;
mod time_argument;

use std::process::ExitCode;

use clap::Parser;
use neomem_core::Source;
use neomem_proto::{Home, Request, Response};

use crate::args::{Cli, Command, ModelsAction};
use crate::connection::DaemonConnection;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(response) => render::print(&response, cli.json),
        Err(error) => {
            eprintln!("neomem: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<Response> {
    let home = Home::from_env().ok_or_else(|| anyhow::anyhow!("neither NEOMEM_HOME nor HOME is set"))?;
    let request = match &cli.command {
        Command::Status => Request::Status,
        Command::Add(arguments) => Request::Add(intake_record::from_arguments(arguments)?),
        Command::Models(arguments) => match &arguments.action {
            None => Request::Models,
            Some(ModelsAction::Test { role, source, structured }) => Request::ModelsTest {
                role: role.model_role(),
                source: source.as_deref().map(Source::new).transpose()?,
                structured: *structured,
            },
        },
        Command::Login(arguments) => return login::run(&home, arguments),
    };
    DaemonConnection::open(&home)?.call(request)
}
