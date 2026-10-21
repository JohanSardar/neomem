mod backend_environment;
mod config;
mod daemon_clock;
mod dispatch;
mod http;
mod instance;
mod mcp;
mod socket;
mod watcher;

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use neomem_engine::{Engine, EngineParts};
use neomem_models::{ReloadingHost, Secrets};
use neomem_proto::Home;

use crate::config::DaemonConfig;
use crate::daemon_clock::DaemonClock;
use crate::dispatch::Dispatcher;
use crate::instance::InstanceLock;

fn main() -> ExitCode {
    init_logging();
    if let Err(error) = backend_environment::apply() {
        tracing::warn!("running without the llama.cpp backend environment: {error}");
    }
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
    let clock = Arc::new(DaemonClock::new(config.dev.settable_clock));
    let engine = Arc::new(open_engine(&home, &config, Arc::clone(&clock))?);
    let sources = config.capture.sources(&home);
    let mut directories = vec![home.root().join("facts"), home.root().join("entities")];
    for source in &sources {
        std::fs::create_dir_all(&source.root)?;
    }
    directories.extend(sources.iter().map(|source| source.root.clone()));
    engine.configure_capture(sources, config.capture.every()?);
    let dispatcher = Arc::new(Dispatcher::new(Arc::clone(&engine), clock));
    http::start(Arc::clone(&dispatcher), &config.http, &home)?;
    engine.run()?;
    let _watcher = watcher::watch(Arc::clone(&engine), &directories)?;
    tracing::info!("neomemd serving {}", home.root().display());
    socket::serve(&dispatcher, &home.socket_path())?;
    drop(lock);
    Ok(())
}

fn open_engine(home: &Home, config: &DaemonConfig, clock: Arc<DaemonClock>) -> anyhow::Result<Engine> {
    let secrets = Secrets::load(&home.secrets_path());
    let models = ReloadingHost::open(home.config_path(), home.models_dir(), secrets, clock.clone())?;
    let recall_config = config.recall_config()?;
    let parts = EngineParts {
        models: Arc::new(models),
        home: home.root().to_path_buf(),
        state_db: home.state_db_path(),
        index_db: home.index_db_path(),
        commit_policy: config.vault.policy()?,
        clock,
        nightly_at: config.schedule.nightly_at()?,
        search_config: config.search,
        recall_config,
        worker: config.worker.config()?,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    Ok(Engine::open(parts)?)
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("NEOMEM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,llama-cpp-2=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).with_ansi(false).init();
}
