use std::fs;
use std::path::Path;

use anyhow::Context;
use neomem_testkit::{FakeModelServer, FakeRules, RecordedCall};

use crate::isolated::Models;
use crate::model_files;

pub const FAKE_SECRETS: &str =
    "NEOMEM_FAKE_JEV_KEY=fake-key\nNEOMEM_FAKE_OPENAI_KEY=fake-key\nNEOMEM_FAKE_ANTHROPIC_KEY=fake-key\n";

pub fn serve(address_file: &Path) -> anyhow::Result<bool> {
    let server = FakeModelServer::bind(0)?;
    let address = server.address().context("the fake model server has no TCP address")?;
    fs::write(address_file, address.to_string())
        .with_context(|| format!("writing {}", address_file.display()))?;
    server.serve_forever();
    Ok(true)
}

pub fn add_rules(address: &str, rules: &FakeRules) -> anyhow::Result<()> {
    ureq::post(&format!("http://{address}/fake/rules")).send_json(rules)?;
    Ok(())
}

pub fn calls(address: &str) -> anyhow::Result<Vec<RecordedCall>> {
    Ok(ureq::get(&format!("http://{address}/fake/calls")).call()?.body_mut().read_json()?)
}

struct LocalAndLiveRoles {
    live_providers: &'static str,
    embedder: String,
    judge: &'static str,
}

fn local_and_live_roles(models: Models) -> LocalAndLiveRoles {
    match models {
        Models::Fake => LocalAndLiveRoles {
            live_providers: "",
            embedder: r#"{ provider = "fake", model = "hash-256" }"#.to_owned(),
            judge: r#"{ provider = "fake_jev" }"#,
        },
        Models::Real => LocalAndLiveRoles {
            live_providers: r#"jev = { kind = "jev", key_env = "JEV_API_KEY", model = "jev-1.13.0" }"#,
            embedder: format!(
                r#"{{ runtime = "llama.cpp", file = "{}" }}"#,
                model_files::EMBEDDINGGEMMA.name
            ),
            judge: r#"{ provider = "jev" }"#,
        },
    }
}

pub fn config_toml(address: &str, fake_llm: &Path, planner: &str, models: Models) -> String {
    let fake_llm = fake_llm.display();
    let LocalAndLiveRoles { live_providers, embedder, judge } = local_and_live_roles(models);
    format!(
        r#"[http]
enabled = true
port = 0

[vault]
commit_every = "1s"

[providers]
fake = {{ kind = "openai", base_url = "http://{address}/v1", key_env = "NEOMEM_FAKE_OPENAI_KEY" }}
fake_responses = {{ kind = "openai", base_url = "http://{address}/v1", key_env = "NEOMEM_FAKE_OPENAI_KEY", api = "responses" }}
fake_anthropic = {{ kind = "anthropic", base_url = "http://{address}", key_env = "NEOMEM_FAKE_ANTHROPIC_KEY" }}
chatgpt = {{ kind = "chatgpt", base_url = "http://{address}/backend-api/codex", issuer = "http://{address}", callback_port = 0 }}
fake_codex = {{ kind = "codex", cmd = ["{fake_llm}", "--brain", "http://{address}", "codex-app-server"] }}
fake_command = {{ kind = "command", cmd = ["{fake_llm}", "--brain", "http://{address}", "command"] }}
fake_jev = {{ kind = "jev", base_url = "http://{address}", key_env = "NEOMEM_FAKE_JEV_KEY", model = "jev-fake" }}
{live_providers}

[models]
embedder = {embedder}
judge = {judge}
checker = {judge}
planner = {{ provider = "{planner}", model = "fake-planner", effort = "low" }}
extractor = {{ provider = "fake", model = "fake-extractor" }}
consolidator = {{ provider = "fake", model = "fake-consolidator" }}

[models.extractor_by_source]
"bank-digest" = {{ provider = "fake_anthropic", model = "claude-fake" }}
"#
    )
}
