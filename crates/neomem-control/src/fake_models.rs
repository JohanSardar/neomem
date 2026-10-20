use std::fs;
use std::path::Path;

use anyhow::Context;
use neomem_testkit::{FakeModelServer, FakeRules, RecordedCall};

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

pub fn config_toml(address: &str) -> String {
    format!(
        r#"[http]
enabled = true
port = 0

[vault]
commit_every = "1s"

[providers]
fake = {{ kind = "openai", base_url = "http://{address}/v1" }}
fake_jev = {{ kind = "jev", base_url = "http://{address}", key_env = "NEOMEM_FAKE_JEV_KEY", model = "jev-fake" }}

[models]
embedder = {{ provider = "fake", model = "hash-256" }}
judge = {{ provider = "fake_jev" }}
checker = {{ provider = "fake_jev" }}
planner = {{ provider = "fake", model = "fake-planner" }}
extractor = {{ provider = "fake", model = "fake-extractor" }}
consolidator = {{ provider = "fake", model = "fake-consolidator" }}
"#
    )
}
