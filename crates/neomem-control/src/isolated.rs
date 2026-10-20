use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use std::os::unix::fs::PermissionsExt;

use neomem_proto::{Client, Envelope, Home, Request, Response};
use neomem_testkit::{FakeRules, RecordedCall};

use crate::binaries::Binaries;
use crate::fake_models;
use crate::proof::{Exchange, ProofRun};

const START_WAIT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct IsolatedNeomem {
    home: Home,
    binaries: Binaries,
    daemon: Option<Child>,
    fake_models: Option<Child>,
    keep_running: bool,
    temporary_dir: Option<tempfile::TempDir>,
    environment: Vec<(String, PathBuf)>,
}

impl IsolatedNeomem {
    pub fn start(binaries: &Binaries) -> anyhow::Result<Self> {
        let mut isolated = Self::prepared(binaries)?;
        isolated.start_daemon()?;
        Ok(isolated)
    }

    pub fn start_at(root: &Path, binaries: &Binaries) -> anyhow::Result<Self> {
        let mut isolated = Self::prepare(root, binaries)?;
        isolated.start_daemon()?;
        Ok(isolated)
    }

    pub fn prepared(binaries: &Binaries) -> anyhow::Result<Self> {
        let temporary = tempfile::Builder::new().prefix("neomemctl-").tempdir()?;
        let mut isolated = Self::prepare(temporary.path(), binaries)?;
        isolated.temporary_dir = Some(temporary);
        Ok(isolated)
    }

    fn prepare(root: &Path, binaries: &Binaries) -> anyhow::Result<Self> {
        refuse_the_real_home(root)?;
        let home = Home::at(root);
        fs::create_dir_all(home.run_dir())?;
        let fake_models = start_fake_models(&home)?;
        let isolated = Self {
            home,
            binaries: binaries.clone(),
            daemon: None,
            fake_models: Some(fake_models),
            keep_running: false,
            temporary_dir: None,
            environment: Vec::new(),
        };
        if !isolated.home.config_path().exists() {
            isolated.plan_with("fake")?;
        }
        fs::write(isolated.home.secrets_path(), fake_models::FAKE_SECRETS)?;
        fs::set_permissions(isolated.home.secrets_path(), fs::Permissions::from_mode(0o600))?;
        Ok(isolated)
    }

    pub fn plan_with(&self, planner_provider: &str) -> anyhow::Result<()> {
        let config =
            fake_models::config_toml(&self.fake_models_address()?, &self.binaries.fake_llm, planner_provider);
        fs::write(self.home.config_path(), config)?;
        Ok(())
    }

    pub fn set_env(&mut self, name: &str, value: &Path) {
        self.environment.push((name.to_owned(), value.to_path_buf()));
    }

    pub fn fake_models_address(&self) -> anyhow::Result<String> {
        Ok(fs::read_to_string(fake_models_address_path(&self.home))?.trim().to_owned())
    }

    pub fn add_fake_rules(&self, rules: &FakeRules) -> anyhow::Result<()> {
        fake_models::add_rules(&self.fake_models_address()?, rules)
    }

    pub fn fake_model_calls(&self) -> anyhow::Result<Vec<RecordedCall>> {
        fake_models::calls(&self.fake_models_address()?)
    }

    pub fn home(&self) -> &Home {
        &self.home
    }

    pub fn binaries(&self) -> &Binaries {
        &self.binaries
    }

    pub fn start_daemon(&mut self) -> anyhow::Result<()> {
        let log = File::options().create(true).append(true).open(self.home.daemon_log_path())?;
        let child = Command::new(&self.binaries.neomemd)
            .env("NEOMEM_HOME", self.home.root())
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .process_group(0)
            .spawn()
            .with_context(|| format!("starting {}", self.binaries.neomemd.display()))?;
        self.daemon = Some(child);
        self.wait_until_serving()
    }

    pub fn wait_until_serving(&self) -> anyhow::Result<()> {
        let deadline = Instant::now() + START_WAIT;
        while Instant::now() < deadline {
            if let Ok(mut client) = Client::connect(&self.home, CALL_TIMEOUT)
                && matches!(client.call(Request::Status), Ok(Response::Status(_)))
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        bail!("neomemd did not start serving within {START_WAIT:?}; log:\n{}", self.logs())
    }

    pub fn client(&self) -> anyhow::Result<Client> {
        Ok(Client::connect(&self.home, CALL_TIMEOUT)?)
    }

    pub fn call(&self, proof: &mut ProofRun, request: Request) -> anyhow::Result<Response> {
        self.send(proof, &Envelope::current(request))
    }

    pub fn send(&self, proof: &mut ProofRun, envelope: &Envelope) -> anyhow::Result<Response> {
        let started = Instant::now();
        let response = self.client()?.send(envelope)?;
        proof.record_exchange(Exchange {
            request: serde_json::to_value(envelope)?,
            response: serde_json::to_value(&response)?,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(response)
    }

    pub fn neomem(&self, arguments: &[&str]) -> anyhow::Result<Output> {
        self.neomem_command(arguments)
            .output()
            .with_context(|| format!("running neomem {}", arguments.join(" ")))
    }

    pub fn spawn_neomem(&self, arguments: &[&str]) -> anyhow::Result<Child> {
        self.neomem_command(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting neomem {}", arguments.join(" ")))
    }

    fn neomem_command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(&self.binaries.neomem);
        command
            .args(arguments)
            .env("NEOMEM_HOME", self.home.root())
            .envs(self.environment.iter().map(|(name, value)| (name, value)));
        command
    }

    pub fn http_address(&self) -> anyhow::Result<String> {
        Ok(fs::read_to_string(self.home.http_address_path())?.trim().to_owned())
    }

    pub fn logs(&self) -> String {
        fs::read_to_string(self.home.daemon_log_path()).unwrap_or_default()
    }

    pub fn stop_daemon(&mut self) -> anyhow::Result<()> {
        if let Ok(mut client) = self.client() {
            let _ = client.call(Request::Shutdown);
        }
        if let Some(mut child) = self.daemon.take() {
            let deadline = Instant::now() + START_WAIT;
            while child.try_wait()?.is_none() {
                if Instant::now() > deadline {
                    child.kill()?;
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }

    pub fn detach(mut self) -> PathBuf {
        self.keep_running = true;
        self.temporary_dir.take().map_or_else(|| self.home.root().to_path_buf(), tempfile::TempDir::keep)
    }
}

impl Drop for IsolatedNeomem {
    fn drop(&mut self) {
        if !self.keep_running {
            let _ = self.stop_daemon();
            if let Some(mut fake_models) = self.fake_models.take() {
                let _ = fake_models.kill();
                let _ = fake_models.wait();
            }
        }
    }
}

pub fn fake_models_pid_path(home: &Home) -> PathBuf {
    home.run_dir().join("fake-models.pid")
}

fn fake_models_address_path(home: &Home) -> PathBuf {
    home.run_dir().join("fake-models.addr")
}

fn start_fake_models(home: &Home) -> anyhow::Result<Child> {
    let address_file = fake_models_address_path(home);
    let _ = fs::remove_file(&address_file);
    let child = Command::new(std::env::current_exe()?)
        .arg("fake-models")
        .arg("--address-file")
        .arg(&address_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .context("starting the fake model server")?;
    fs::write(fake_models_pid_path(home), child.id().to_string())?;
    let deadline = Instant::now() + START_WAIT;
    while !address_file.exists() {
        if Instant::now() > deadline {
            bail!("the fake model server did not start");
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(child)
}

fn refuse_the_real_home(root: &Path) -> anyhow::Result<()> {
    let Some(user_home) = std::env::var_os("HOME") else { return Ok(()) };
    let real = Path::new(&user_home).join(".neomem");
    let same = match (fs::canonicalize(root), fs::canonicalize(&real)) {
        (Ok(candidate), Ok(real)) => candidate == real,
        _ => root == real,
    };
    if same {
        bail!("neomemctl never runs against the real ~/.neomem");
    }
    Ok(())
}
