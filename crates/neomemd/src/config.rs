use std::fs;

use anyhow::Context;
use neomem_engine::{CommitPolicy, NightlyAt};
use neomem_models::{ModelsConfig, parse_duration};
use neomem_proto::Home;
use serde::Deserialize;

const DEFAULT_HTTP_PORT: u16 = 47_391;

const DEFAULT_CONFIG: &str = r#"[http]
enabled = true
port = 47391

[vault]
commit_every = "30s"
commit_after_changes = 50

[schedule]
nightly_at = "03:00"
"#;

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub http: HttpConfig,
    #[serde(default)]
    pub vault: VaultConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub dev: DevConfig,
    #[serde(flatten)]
    pub models: ModelsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HttpConfig {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DevConfig {
    #[serde(default)]
    pub settable_clock: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    #[serde(default = "default_nightly_at")]
    pub nightly_at: String,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self { nightly_at: default_nightly_at() }
    }
}

fn default_nightly_at() -> String {
    "03:00".to_owned()
}

impl ScheduleConfig {
    pub fn nightly_at(&self) -> anyhow::Result<NightlyAt> {
        NightlyAt::parse(&self.nightly_at)
            .ok_or_else(|| anyhow::anyhow!("[schedule] nightly_at `{}` is not HH:MM", self.nightly_at))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VaultConfig {
    #[serde(default = "default_commit_every")]
    pub commit_every: String,
    #[serde(default = "default_commit_after_changes")]
    pub commit_after_changes: usize,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self { commit_every: default_commit_every(), commit_after_changes: default_commit_after_changes() }
    }
}

fn default_commit_every() -> String {
    "30s".to_owned()
}

fn default_commit_after_changes() -> usize {
    50
}

impl VaultConfig {
    pub fn policy(&self) -> anyhow::Result<CommitPolicy> {
        let every = parse_duration(&self.commit_every).ok_or_else(|| {
            anyhow::anyhow!("[vault] commit_every `{}` is not a duration", self.commit_every)
        })?;
        Ok(CommitPolicy { every, after_changes: self.commit_after_changes })
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self { enabled: true, port: DEFAULT_HTTP_PORT }
    }
}

fn enabled_by_default() -> bool {
    true
}

fn default_port() -> u16 {
    DEFAULT_HTTP_PORT
}

impl DaemonConfig {
    pub fn load_or_create(home: &Home) -> anyhow::Result<Self> {
        let path = home.config_path();
        if !path.exists() {
            fs::write(&path, DEFAULT_CONFIG).with_context(|| format!("writing {}", path.display()))?;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}
