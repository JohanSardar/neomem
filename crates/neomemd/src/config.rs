use std::fs;

use anyhow::Context;
use neomem_models::ModelsConfig;
use neomem_proto::Home;
use serde::Deserialize;

const DEFAULT_HTTP_PORT: u16 = 47_391;

const DEFAULT_CONFIG: &str = r"[http]
enabled = true
port = 47391
";

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub http: HttpConfig,
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
