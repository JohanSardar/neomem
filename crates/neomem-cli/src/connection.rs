use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use neomem_proto::{Client, Home, Request, Response};

use crate::service::Service;
use std::os::unix::process::CommandExt;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const START_WAIT: Duration = Duration::from_secs(2);
const START_POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub struct DaemonConnection {
    home: Home,
    client: Client,
}

impl DaemonConnection {
    pub fn existing(home: &Home) -> anyhow::Result<Self> {
        Ok(Self { home: home.clone(), client: Client::connect(home, REQUEST_TIMEOUT)? })
    }

    pub fn open(home: &Home) -> anyhow::Result<Self> {
        let client = match Client::connect(home, REQUEST_TIMEOUT) {
            Ok(client) => client,
            Err(_) => start_and_connect(home)?,
        };
        Ok(Self { home: home.clone(), client })
    }

    pub fn waiting_up_to(self, timeout: Duration) -> anyhow::Result<Self> {
        self.client.set_timeout(timeout)?;
        Ok(self)
    }

    pub fn call(mut self, request: Request) -> anyhow::Result<Response> {
        let response = self.client.call(request.clone())?;
        if let Response::VersionMismatch { .. } = response {
            restart(&self.home, &mut self.client)?;
            let mut fresh = Client::connect(&self.home, REQUEST_TIMEOUT)?;
            return Ok(fresh.call(request)?);
        }
        Ok(response)
    }
}

fn restart(home: &Home, client: &mut Client) -> anyhow::Result<()> {
    let _ = client.call(Request::Shutdown);
    wait_until_gone(home);
    start_and_connect(home).map(drop)
}

pub(crate) fn stop_daemon(home: &Home) {
    if let Ok(mut client) = Client::connect(home, REQUEST_TIMEOUT) {
        let _ = client.call(Request::Shutdown);
        wait_until_gone(home);
    }
}

fn wait_until_gone(home: &Home) {
    let deadline = Instant::now() + START_WAIT;
    while Client::connect(home, REQUEST_TIMEOUT).is_ok() && Instant::now() < deadline {
        thread::sleep(START_POLL);
    }
}

fn start_and_connect(home: &Home) -> anyhow::Result<Client> {
    if !Service::serving(home).is_some_and(|service| service.start()) {
        spawn_daemon(home)?;
    }
    let deadline = Instant::now() + START_WAIT;
    loop {
        match Client::connect(home, REQUEST_TIMEOUT) {
            Ok(client) => return Ok(client),
            Err(_) if Instant::now() < deadline => thread::sleep(START_POLL),
            Err(error) => bail!("neomemd did not start within 2 seconds: {error}"),
        }
    }
}

fn spawn_daemon(home: &Home) -> anyhow::Result<()> {
    std::fs::create_dir_all(home.run_dir()).context("creating the run directory")?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.daemon_log_path())
        .context("opening the daemon log")?;
    Command::new(daemon_binary())
        .env("NEOMEM_HOME", home.root())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()
        .context("starting neomemd")?;
    Ok(())
}

pub(crate) fn daemon_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("neomemd"))
        .filter(|sibling| sibling.exists())
        .unwrap_or_else(|| PathBuf::from("neomemd"))
}
