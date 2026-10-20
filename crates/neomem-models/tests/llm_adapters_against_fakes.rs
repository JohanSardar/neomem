#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Instant;

use neomem_core::{Llm, LlmMessage, LlmRequest, LlmRole, ModelRoles, Source};
use neomem_models::{ModelHost, ModelsConfig, Secrets};
use neomem_testkit::{
    FAKE_ACCOUNT_ID, FAKE_EMAIL, FakeModelServer, FakeRules, LlmRule, RecordedCall, fake_llm_binary,
    llm_contract,
};
use serde_json::Value;

struct Fake {
    address: String,
    secrets_dir: tempfile::TempDir,
}

impl Fake {
    fn start() -> Self {
        let address = FakeModelServer::bind(0).unwrap().serve_in_background().unwrap().to_string();
        let secrets_dir = tempfile::tempdir().unwrap();
        let key = ["fake", "-", "key"].concat();
        std::fs::write(secrets_dir.path().join("secrets.env"), format!("NEOMEM_TEST_FAKE_KEY={key}\n"))
            .unwrap();
        Self { address, secrets_dir }
    }

    fn host(&self, provider: &str, role_extra: &str) -> ModelHost {
        let address = &self.address;
        let fake_llm = fake_llm_binary().display().to_string();
        let config = format!(
            r#"
[providers]
chat = {{ kind = "openai", base_url = "http://{address}/v1", key_env = "NEOMEM_TEST_FAKE_KEY" }}
responses = {{ kind = "openai", base_url = "http://{address}/v1", api = "responses", key_env = "NEOMEM_TEST_FAKE_KEY" }}
anthropic = {{ kind = "anthropic", base_url = "http://{address}", key_env = "NEOMEM_TEST_FAKE_KEY" }}
codex = {{ kind = "codex", cmd = ["{fake_llm}", "--brain", "http://{address}", "codex-app-server"] }}
command = {{ kind = "command", cmd = ["{fake_llm}", "--brain", "http://{address}", "command"] }}
chatgpt = {{ kind = "chatgpt", base_url = "http://{address}/backend-api/codex", issuer = "http://{address}", callback_port = 0 }}

[models]
planner = {{ provider = "{provider}", model = "fake-model"{role_extra} }}
extractor = {{ provider = "chat", model = "fake-extractor" }}

[models.extractor_by_source]
"bank-digest" = {{ provider = "anthropic", model = "claude-fake" }}
"#
        );
        let config: ModelsConfig = toml::from_str(&config).unwrap();
        let secrets = Secrets::load(&self.secrets_dir.path().join("secrets.env"));
        ModelHost::new(config, secrets, self.secrets_dir.path().to_path_buf())
    }

    fn planner(&self, provider: &str, role_extra: &str) -> Arc<dyn Llm> {
        self.host(provider, role_extra).llm(LlmRole::Planner, None).unwrap()
    }

    fn add_rule(&self, rule: LlmRule) {
        let rules = FakeRules { llm: vec![rule], ..FakeRules::default() };
        ureq::post(&format!("http://{}/fake/rules", self.address)).send_json(&rules).unwrap();
    }

    fn calls(&self) -> Vec<RecordedCall> {
        ureq::get(&format!("http://{}/fake/calls", self.address))
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap()
    }
}

fn ping() -> LlmRequest {
    LlmRequest {
        system: "Plan the search.".to_owned(),
        messages: vec![LlmMessage::User("ping".to_owned())],
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: 32,
    }
}

#[test]
fn the_openai_chat_adapter_passes_the_llm_contract() {
    llm_contract(Fake::start().planner("chat", "").as_ref());
}

#[test]
fn the_openai_responses_adapter_passes_the_llm_contract() {
    llm_contract(Fake::start().planner("responses", "").as_ref());
}

#[test]
fn the_anthropic_adapter_passes_the_llm_contract() {
    llm_contract(Fake::start().planner("anthropic", ", effort = \"high\"").as_ref());
}

#[test]
fn the_chatgpt_adapter_signs_in_with_pkce_refreshes_and_passes_the_llm_contract() {
    let fake = Fake::start();
    let host = fake.host("chatgpt", ", effort = \"low\"");
    let unsigned = host.llm(LlmRole::Planner, None).unwrap().complete(&ping(), &mut |_| {});
    assert!(unsigned.unwrap_err().to_string().contains("neomem login chatgpt"));
    let url = host.start_login("chatgpt").unwrap();
    assert!(url.contains("code_challenge_method=S256") && url.contains("redirect_uri=http"), "{url}");
    assert_eq!(host.start_login("chatgpt").unwrap(), url, "a pending sign-in is reused");
    assert_eq!(ureq::get(&url).call().unwrap().status(), 200, "the browser lands on the signed-in page");
    assert_eq!(host.finish_login("chatgpt").unwrap(), FAKE_EMAIL);
    let secrets = std::fs::read_to_string(fake.secrets_dir.path().join("secrets.env")).unwrap();
    assert!(secrets.contains("NEOMEM_TEST_FAKE_KEY="), "other secrets stay");
    assert!(secrets.contains(&format!("NEOMEM_CHATGPT_ACCOUNT_ID={FAKE_ACCOUNT_ID}")));
    llm_contract(host.llm(LlmRole::Planner, None).unwrap().as_ref());
    let calls = fake.calls();
    let refreshes = calls.iter().filter(|call| call.body["grant_type"] == "refresh_token").count();
    assert_eq!(refreshes, 1, "the short first token is refreshed once and the new one is kept");
    let backend = calls.iter().find(|call| call.route == "/backend-api/codex/responses").unwrap();
    assert_eq!(backend.body["reasoning"]["effort"], "low");
    assert_eq!(backend.body["store"], false);
}

#[test]
fn the_codex_adapter_passes_the_llm_contract() {
    llm_contract(Fake::start().planner("codex", "").as_ref());
}

#[test]
fn the_command_adapter_passes_the_llm_contract() {
    llm_contract(Fake::start().planner("command", "").as_ref());
}

#[test]
fn the_codex_app_server_stays_warm_between_calls() {
    let fake = Fake::start();
    let codex = fake.planner("codex", "");
    codex.complete(&ping(), &mut |_| {}).unwrap();
    codex.complete(&ping(), &mut |_| {}).unwrap();
    let threads: Vec<Value> =
        fake.calls().into_iter().map(|call| call.body["wire"]["threadId"].clone()).collect();
    assert_eq!(threads, ["thr_fake_1", "thr_fake_3"], "both turns ran in the same app-server process");
}

#[test]
fn every_wire_format_streams_text_before_the_reply_completes() {
    for provider in ["chat", "responses", "anthropic", "codex"] {
        let fake = Fake::start();
        fake.add_rule(LlmRule {
            system_contains: None,
            user_contains: Some("ping".to_owned()),
            reply: "one two three four".to_owned(),
            delay_ms: 0,
            delta_delay_ms: 80,
            times: None,
        });
        let started = Instant::now();
        let mut first = None;
        let reply = fake.planner(provider, "").complete(&ping(), &mut |_| {
            first.get_or_insert_with(|| started.elapsed());
        });
        let finished = started.elapsed();
        assert_eq!(reply.unwrap().text, "one two three four", "{provider}");
        assert!(first.unwrap() + std::time::Duration::from_millis(200) < finished, "{provider} streams");
    }
}

#[test]
fn effort_and_caching_reach_each_provider_in_its_own_form() {
    let fake = Fake::start();
    for provider in ["chat", "responses", "anthropic", "codex", "command"] {
        fake.planner(provider, ", effort = \"low\"").complete(&ping(), &mut |_| {}).unwrap();
    }
    let bodies: Vec<Value> = fake.calls().into_iter().map(|call| call.body).collect();
    assert_eq!(bodies[0]["reasoning_effort"], "low");
    assert_eq!(bodies[1]["reasoning"]["effort"], "low");
    assert_eq!(bodies[1]["store"], false);
    assert_eq!(bodies[2]["output_config"]["effort"], "low");
    assert_eq!(bodies[3]["wire"]["effort"], "low");
    assert_eq!(bodies[3]["via"], "codex-app-server");
    assert_eq!(bodies[4]["wire"]["effort"], "low");
    assert_eq!(bodies[4]["via"], "command");
    assert_eq!(bodies[2]["system"][0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn anthropic_thinking_never_reaches_the_text() {
    let fake = Fake::start();
    let reply = fake.planner("anthropic", ", effort = \"medium\"").complete(&ping(), &mut |_| {}).unwrap();
    assert_eq!(reply.text, "OK");
}

#[test]
fn the_extractor_by_source_override_gets_its_own_provider() {
    let host = Fake::start().host("chat", "");
    let bank = Source::new("bank-digest").unwrap();
    let other = Source::new("oura-digest").unwrap();
    assert_eq!(
        host.llm(LlmRole::Extractor, Some(&bank)).unwrap().model_id().as_str(),
        "anthropic:claude-fake"
    );
    assert_eq!(
        host.llm(LlmRole::Extractor, Some(&other)).unwrap().model_id().as_str(),
        "openai:fake-extractor"
    );
    assert_eq!(host.llm(LlmRole::Extractor, None).unwrap().model_id().as_str(), "openai:fake-extractor");
}

#[test]
fn a_missing_anthropic_key_is_reported_by_name() {
    let fake = Fake::start();
    let config: ModelsConfig = toml::from_str(
        "[providers]\nanthropic = { kind = \"anthropic\", key_env = \"NEOMEM_TEST_UNSET_KEY\" }\n[models]\nplanner = { provider = \"anthropic\", model = \"m\" }\n",
    )
    .unwrap();
    let host = ModelHost::new(config, Secrets::default(), fake.secrets_dir.path().to_path_buf());
    let error = host.llm(LlmRole::Planner, None).unwrap_err().to_string();
    assert!(error.contains("NEOMEM_TEST_UNSET_KEY"), "{error}");
}
