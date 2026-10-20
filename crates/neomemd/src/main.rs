mod config;
mod dispatch;
mod http;
mod instance;
mod socket;
mod system_clock;
mod watcher;

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use neomem_engine::{Engine, EngineParts};
use neomem_models::{ModelHost, Secrets};
use neomem_proto::Home;

use crate::config::DaemonConfig;
use crate::instance::InstanceLock;
use crate::system_clock::SystemClock;

fn main() -> ExitCode {
    init_logging();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("neomemd stopped: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let home = Home::from_env().context("neither NEOMEM_HOME nor HOME is set")?;
    instance::prepare_home(&home)?;
    let Some(lock) = InstanceLock::acquire(&home)? else {
        tracing::info!("another neomemd already serves {}", home.root().display());
        return Ok(());
    };
    let config = DaemonConfig::load_or_create(&home)?;
    let engine = Arc::new(open_engine(&home, &config)?);
    http::start(Arc::clone(&engine), &config.http, &home)?;
    engine.run()?;
    let _watcher =
        watcher::watch(Arc::clone(&engine), &[home.root().join("facts"), home.root().join("entities")])?;
    tracing::info!("neomemd serving {}", home.root().display());
    socket::serve(&engine, &home.socket_path())?;
    drop(lock);
    Ok(())
}

fn open_engine(home: &Home, config: &DaemonConfig) -> anyhow::Result<Engine> {
    let secrets = Secrets::load(&home.secrets_path());
    let models = ModelHost::new(config.models.clone(), secrets, home.models_dir());
    let parts = EngineParts {
        models: Arc::new(models),
        home: home.root().to_path_buf(),
        state_db: home.state_db_path(),
        commit_policy: config.vault.policy()?,
        clock: Arc::new(SystemClock),
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    Ok(Engine::open(parts)?)
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("NEOMEM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).with_ansi(false).init();
}
