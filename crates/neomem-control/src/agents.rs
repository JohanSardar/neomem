use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use tempfile::TempDir;

use crate::isolated::IsolatedNeomem;

const SYSTEMD: &str = "/usr/lib/systemd/systemd";
const EMPTY_TARGET: &str = "neomem-proof.target";
const MANAGER_WAIT: Duration = Duration::from_secs(5);
const FAKE_KEY: &str = "fake-agent-key";
const AGENT_PROGRAMS: &[&str] = &["claude", "codex", "pi", "node"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
    Pi,
}

impl Agent {
    pub const ALL: [Self; 3] = [Self::ClaudeCode, Self::Codex, Self::Pi];

    pub fn program(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }

    pub fn source(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }

    pub fn system_marker(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Agent SDK",
            Self::Codex => "running in the Codex CLI",
            Self::Pi => "operating inside pi",
        }
    }

    pub fn recall_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "cc",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }
}

#[derive(Debug)]
pub struct AgentSandbox {
    root: TempDir,
    neomem_home: PathBuf,
    neomem_binaries: PathBuf,
    fake_models: String,
    manager: Option<Child>,
}

impl AgentSandbox {
    pub fn new(neomem: &IsolatedNeomem) -> anyhow::Result<Self> {
        let root = tempfile::Builder::new().prefix("neomem-agents-").tempdir()?;
        let neomem_home = neomem.home().root().to_path_buf();
        let neomem_binaries =
            neomem.binaries().neomem.parent().context("the neomem binary has no directory")?.to_path_buf();
        let mut sandbox = Self {
            root,
            neomem_home,
            neomem_binaries,
            fake_models: neomem.fake_models_address()?,
            manager: None,
        };
        for directory in
            [sandbox.home(), sandbox.runtime(), sandbox.work(), sandbox.claude_dir(), sandbox.pi_dir()]
        {
            fs::create_dir_all(directory)?;
        }
        fs::set_permissions(sandbox.runtime(), fs::Permissions::from_mode(0o700))?;
        sandbox.configure_fake_providers()?;
        sandbox.manager = sandbox.start_private_systemd()?;
        Ok(sandbox)
    }

    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn runtime(&self) -> PathBuf {
        self.root.path().join("run")
    }

    pub fn work(&self) -> PathBuf {
        self.root.path().join("work")
    }

    pub fn claude_dir(&self) -> PathBuf {
        self.neomem_home.join("capture/cc")
    }

    pub fn codex_dir(&self) -> PathBuf {
        self.neomem_home.join("capture/codex")
    }

    pub fn pi_dir(&self) -> PathBuf {
        self.neomem_home.join("capture/pi")
    }

    pub fn has_service_manager(&self) -> bool {
        self.manager.is_some()
    }

    fn configure_fake_providers(&self) -> anyhow::Result<()> {
        let address = &self.fake_models;
        fs::create_dir_all(self.codex_dir())?;
        fs::write(
            self.codex_dir().join("config.toml"),
            format!(
                "model = \"fake-model\"\nmodel_provider = \"fake\"\n\n[model_providers.fake]\nname = \"fake\"\nbase_url = \"http://{address}/v1\"\nwire_api = \"responses\"\nenv_key = \"FAKE_MODEL_KEY\"\n"
            ),
        )?;
        let models = serde_json::json!({ "providers": { "fake": {
            "baseUrl": format!("http://{address}/v1"), "api": "openai-completions", "apiKey": "FAKE_MODEL_KEY",
            "models": [{ "id": "fake-model" }] } } });
        fs::write(self.pi_dir().join("models.json"), serde_json::to_string_pretty(&models)?)?;
        Ok(())
    }

    fn start_private_systemd(&self) -> anyhow::Result<Option<Child>> {
        if !cfg!(target_os = "linux") || !Path::new(SYSTEMD).exists() {
            return Ok(None);
        }
        let units = self.home().join(".config/systemd/user");
        fs::create_dir_all(&units)?;
        fs::write(units.join(EMPTY_TARGET), "[Unit]\nDescription=neomem proof sandbox\n")?;
        let manager = Command::new(SYSTEMD)
            .arg("--user")
            .arg(format!("--unit={EMPTY_TARGET}"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home())
            .env("XDG_RUNTIME_DIR", self.runtime())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting a private systemd user manager")?;
        let socket = self.runtime().join("systemd/private");
        let deadline = Instant::now() + MANAGER_WAIT;
        while !socket.exists() {
            if Instant::now() > deadline {
                bail!("the private systemd user manager did not start");
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(Some(manager))
    }

    fn path(&self) -> anyhow::Result<String> {
        let mut directories = vec![self.neomem_binaries.clone()];
        let inherited = env::var_os("PATH").unwrap_or_default();
        for program in AGENT_PROGRAMS {
            if let Some(directory) =
                env::split_paths(&inherited).find(|directory| directory.join(program).is_file())
                && !directories.contains(&directory)
            {
                directories.push(directory);
            }
        }
        directories.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
        Ok(env::join_paths(directories)?.to_string_lossy().into_owned())
    }

    pub fn command(&self, program: &str) -> anyhow::Result<Command> {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("PATH", self.path()?)
            .env("HOME", self.home())
            .env("XDG_RUNTIME_DIR", self.runtime())
            .env("NEOMEM_HOME", &self.neomem_home)
            .env("CLAUDE_CONFIG_DIR", self.claude_dir())
            .env("CODEX_HOME", self.codex_dir())
            .env("PI_CODING_AGENT_DIR", self.pi_dir())
            .env("ANTHROPIC_BASE_URL", format!("http://{}", self.fake_models))
            .env("ANTHROPIC_API_KEY", FAKE_KEY)
            .env("FAKE_MODEL_KEY", FAKE_KEY)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("DISABLE_AUTOUPDATER", "1")
            .env("PI_OFFLINE", "1")
            .current_dir(self.work())
            .stdin(Stdio::null());
        Ok(command)
    }

    pub fn run(&self, program: &str, arguments: &[&str]) -> anyhow::Result<Output> {
        self.command(program)?
            .args(arguments)
            .output()
            .with_context(|| format!("running {program} {}", arguments.join(" ")))
    }

    pub fn session(&self, agent: Agent, prompt: &str) -> anyhow::Result<Output> {
        match agent {
            Agent::ClaudeCode => self.run("claude", &["-p", prompt]),
            Agent::Codex => self.run("codex", &["exec", "--skip-git-repo-check", prompt]),
            Agent::Pi => self.run("pi", &["-p", "--provider", "fake", "--model", "fake-model", prompt]),
        }
    }
}

impl Drop for AgentSandbox {
    fn drop(&mut self) {
        if let Some(mut manager) = self.manager.take() {
            let _ = self.run("systemctl", &["--user", "exit"]);
            let deadline = Instant::now() + MANAGER_WAIT;
            while manager.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let _ = manager.kill();
            let _ = manager.wait();
        }
        let _ = Command::new("chmod").arg("-R").arg("u+rwx").arg(self.root.path()).status();
    }
}
