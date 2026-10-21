use std::collections::BTreeMap;

use neomem_core::Effort;
use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ModelsConfig {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub models: RolesConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Chatgpt,
    Codex,
    Command,
    Jev,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: Option<String>,
    pub key_env: Option<String>,
    pub model: Option<String>,
    pub api: Option<String>,
    pub cmd: Option<Vec<String>>,
    pub issuer: Option<String>,
    pub callback_port: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RolesConfig {
    pub embedder: Option<RoleConfig>,
    pub judge: Option<RoleConfig>,
    pub planner: Option<RoleConfig>,
    pub extractor: Option<RoleConfig>,
    pub checker: Option<RoleConfig>,
    pub consolidator: Option<RoleConfig>,
    pub idle_unload: Option<String>,
    #[serde(default)]
    pub extractor_by_source: BTreeMap<String, RoleConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RoleConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub timeout: Option<String>,
    pub runtime: Option<String>,
    pub file: Option<String>,
    pub fallback: Option<Box<RoleConfig>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_EXAMPLE: &str = r#"
[providers]
openai      = { kind = "openai", base_url = "https://api.openai.com/v1", key_env = "OPENAI_API_KEY", api = "responses" }
jev         = { kind = "jev", key_env = "JEV_API_KEY", model = "jev-1.13.0" }
chatgpt     = { kind = "chatgpt" }
mycli       = { kind = "command", cmd = ["my-llm-cli", "--json"] }

[models]
embedder     = { runtime = "llama.cpp", file = "embeddinggemma-300m-Q8_0.gguf" }
judge        = { provider = "jev", fallback = { runtime = "llama.cpp", file = "qwen3-reranker-0.6b-Q8_0.gguf" } }
planner      = { provider = "chatgpt", model = "gpt-6.1-sol", effort = "low", timeout = "2.5s" }
checker      = { provider = "jev" }
idle_unload  = "15m"

[models.extractor_by_source]
"bank-digest" = { provider = "ollama", model = "qwen3:8b" }
"#;

    #[test]
    fn the_spec_example_parses() {
        let config: ModelsConfig = toml::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(config.providers["jev"].kind, ProviderKind::Jev);
        let judge = config.models.judge.unwrap();
        assert_eq!(judge.fallback.unwrap().runtime.as_deref(), Some("llama.cpp"));
        assert_eq!(config.models.planner.unwrap().effort, Some(Effort::Low));
        assert!(config.models.extractor_by_source.contains_key("bank-digest"));
    }
}
