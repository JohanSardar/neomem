use std::fs;

use anyhow::Context;
use neomem_core::SearchConfig;
use neomem_engine::{CommitPolicy, NightlyAt, WorkerConfig};
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
    pub recall: RecallSection,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub vault: VaultConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub worker: WorkerSection,
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
pub struct WorkerSection {
    pub session_silence: Option<String>,
    pub batch_every: Option<String>,
    pub batch_size: Option<usize>,
    pub lease: Option<String>,
}

impl WorkerSection {
    pub fn config(&self) -> anyhow::Result<WorkerConfig> {
        let defaults = WorkerConfig::default();
        let duration = |text: &Option<String>, fallback, name: &str| -> anyhow::Result<std::time::Duration> {
            text.as_deref().map_or(Ok(fallback), |text| {
                parse_duration(text)
                    .ok_or_else(|| anyhow::anyhow!("[worker] {name} `{text}` is not a duration"))
            })
        };
        Ok(WorkerConfig {
            session_silence: duration(&self.session_silence, defaults.session_silence, "session_silence")?,
            batch_every: duration(&self.batch_every, defaults.batch_every, "batch_every")?,
            batch_size: self.batch_size.unwrap_or(defaults.batch_size),
            lease: duration(&self.lease, defaults.lease, "lease")?,
        })
    }
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecallSection {
    pub hard_timeout: Option<String>,
}

impl DaemonConfig {
    pub fn recall_config(&self) -> anyhow::Result<neomem_engine::RecallConfig> {
        let defaults = neomem_engine::RecallConfig::default();
        let parse = |text: Option<&str>, default| {
            text.map_or(Ok(default), |text| {
                parse_duration(text).ok_or_else(|| anyhow::anyhow!("invalid recall duration"))
            })
        };
        Ok(neomem_engine::RecallConfig {
            planner_timeout: parse(
                self.models.models.planner.as_ref().and_then(|role| role.timeout.as_deref()),
                defaults.planner_timeout,
            )?,
            hard_timeout: parse(self.recall.hard_timeout.as_deref(), defaults.hard_timeout)?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureConfig {
    pub claude_dir: Option<std::path::PathBuf>,
    pub codex_dir: Option<std::path::PathBuf>,
    pub pi_dir: Option<std::path::PathBuf>,
    #[serde(default = "default_capture_sweep")]
    pub sweep_every: String,
}

fn default_capture_sweep() -> String {
    "60s".to_owned()
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self { claude_dir: None, codex_dir: None, pi_dir: None, sweep_every: default_capture_sweep() }
    }
}

impl CaptureConfig {
    pub fn sources(&self) -> Vec<neomem_engine::CaptureSource> {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        vec![
            neomem_engine::CaptureSource {
                root: self.claude_dir.clone().unwrap_or_else(|| home.join(".claude")).join("projects"),
                adapter: std::sync::Arc::new(neomem_capture::ClaudeCode),
            },
            neomem_engine::CaptureSource {
                root: self.codex_dir.clone().unwrap_or_else(|| home.join(".codex")).join("sessions"),
                adapter: std::sync::Arc::new(neomem_capture::Codex),
            },
            neomem_engine::CaptureSource {
                root: self.pi_dir.clone().unwrap_or_else(|| home.join(".pi/agent")).join("sessions"),
                adapter: std::sync::Arc::new(neomem_capture::Pi),
            },
        ]
    }

    pub fn every(&self) -> anyhow::Result<std::time::Duration> {
        parse_duration(&self.sweep_every)
            .filter(|every| !every.is_zero())
            .ok_or_else(|| anyhow::anyhow!("capture.sweep_every must be a positive duration"))
    }
}
