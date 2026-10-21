use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub base_url: String,
    pub model: String,
    pub version: String,
    #[serde(default)]
    pub key_env: Option<String>,
    pub max_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    pub daemon_config: String,
    pub models_dir: Option<std::path::PathBuf>,
    pub reader: Endpoint,
    pub answer_judge: Endpoint,
    pub context_budget: usize,
    pub provenance: Value,
}

impl Endpoint {
    pub fn complete(&self, system: &str, user: &str) -> anyhow::Result<(String, u64)> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_mins(30)))
            .build()
            .into();
        let mut request = agent.post(&format!("{}/chat/completions", self.base_url.trim_end_matches('/')));
        if let Some(name) = &self.key_env {
            request = request.header(
                "Authorization",
                &format!("Bearer {}", std::env::var(name).context("missing model key")?),
            );
        }
        let response: Value = request
            .send_json(json!({
                "model": self.model, "temperature": 0, "seed": 20_261_021,
                "max_tokens": self.max_tokens,
                "messages": [{"role":"system","content":system},{"role":"user","content":user}]
            }))?
            .body_mut()
            .read_json()?;
        let answer =
            response["choices"][0]["message"]["content"].as_str().context("missing model answer")?.to_owned();
        let tokens =
            response["usage"]["prompt_tokens"].as_u64().context("provider must return prompt token usage")?;
        ensure!(!answer.trim().is_empty(), "empty model answer");
        Ok((answer, tokens))
    }
}
