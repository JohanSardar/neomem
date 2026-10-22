use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};
use neomem_proto::{Client, Home, Request, Response};

#[derive(Debug)]
pub struct Daemon {
    pub home: Home,
    pub exchanges: std::sync::Mutex<Vec<serde_json::Value>>,
    child: Child,
    _temporary: tempfile::TempDir,
}

pub fn sibling(name: &str) -> anyhow::Result<std::path::PathBuf> {
    Ok(std::env::current_exe()?.parent().context("executable directory")?.join(name))
}

impl Daemon {
    pub fn start(config: &str, models: Option<&Path>, sign_in: Option<&Path>) -> anyhow::Result<Self> {
        let temporary = tempfile::Builder::new().prefix("neomem-eval-").tempdir()?;
        let home = Home::at(temporary.path());
        fs::create_dir_all(home.run_dir())?;
        let root = home.root().display();
        let isolation = format!(
            "\n[capture]\nclaude_dir = \"{root}/capture/cc\"\ncodex_dir = \"{root}/capture/codex\"\npi_dir = \"{root}/capture/pi\"\nsweep_every = \"250ms\"\n[http]\nenabled = false\n[worker]\nsession_silence = \"1s\"\nbatch_every = \"1s\"\nbatch_size = 100000\n"
        );
        let mut parsed: toml::Value = toml::from_str(config)?;
        let overrides: toml::Value = toml::from_str(&isolation)?;
        let table = parsed.as_table_mut().context("config table")?;
        for (key, value) in overrides.as_table().context("isolation table")? {
            table.insert(key.clone(), value.clone());
        }
        fs::write(home.config_path(), toml::to_string(&parsed)?)?;
        if let Some(models) = models {
            std::os::unix::fs::symlink(models, home.models_dir())?;
        }
        if let Some(sign_in) = sign_in {
            fs::copy(sign_in, home.secrets_path()).context("copying the shared sign-in")?;
        }
        let child = Command::new(sibling("neomemd")?)
            .env("NEOMEM_HOME", home.root())
            .env("TZ", "UTC")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(home.daemon_log_path())?)
            .spawn()?;
        let daemon = Self { home, child, _temporary: temporary, exchanges: std::sync::Mutex::default() };
        let deadline = Instant::now() + Duration::from_mins(3);
        loop {
            if matches!(daemon.call(Request::Status), Ok(Response::Status(_))) {
                return Ok(daemon);
            }
            ensure!(
                Instant::now() < deadline,
                "daemon startup timeout: {}",
                fs::read_to_string(daemon.home.daemon_log_path())?
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn return_sign_in(&self, sign_in: Option<&Path>) -> anyhow::Result<()> {
        let Some(sign_in) = sign_in else { return Ok(()) };
        let used = fs::read(self.home.secrets_path())?;
        if used != fs::read(sign_in)? {
            let staging = sign_in.with_extension("env.returning");
            fs::copy(self.home.secrets_path(), &staging)?;
            fs::rename(&staging, sign_in)?;
        }
        Ok(())
    }

    pub fn call(&self, request: Request) -> anyhow::Result<Response> {
        let started = Instant::now();
        let envelope = serde_json::to_value(&request)?;
        let response = Client::connect(&self.home, Duration::from_mins(30))?.call(request)?;
        self.exchanges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(serde_json::json!({"request":envelope,"response":response,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}));
        if let Response::Failed(failure) = &response {
            bail!("daemon: {}", failure.message);
        }
        Ok(response)
    }

    pub fn finish(&self) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_hours(2);
        loop {
            if let Response::Status(status) = self.call(Request::Status)? {
                let (pending, failed) =
                    status.extraction.context("daemon does not report extraction state")?;
                ensure!(failed == 0, "{failed} extraction jobs failed");
                if pending == 0 {
                    break;
                }
            }
            ensure!(Instant::now() < deadline, "extraction timeout");
            thread::sleep(Duration::from_millis(200));
        }
        ensure!(matches!(self.call(Request::Dream)?, Response::Dreamed(_)), "consolidation did not finish");
        Ok(())
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.call(Request::Shutdown);
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
