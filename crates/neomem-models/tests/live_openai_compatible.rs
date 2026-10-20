#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use neomem_core::{LlmMessage, LlmRequest, LlmRole, ModelRoles};
use neomem_models::{ModelHost, ModelsConfig, Secrets};
use neomem_testkit::llm_contract;

#[test]
#[ignore = "calls a live OpenAI-compatible server; run with NEOMEM_LIVE_OPENAI_BASE_URL set (llama-server works)"]
fn a_live_openai_compatible_server_passes_the_llm_contract_and_streams() {
    let base_url = std::env::var("NEOMEM_LIVE_OPENAI_BASE_URL").expect("NEOMEM_LIVE_OPENAI_BASE_URL is set");
    let model = std::env::var("NEOMEM_LIVE_OPENAI_MODEL").unwrap_or_else(|_| "local".to_owned());
    let config = format!(
        "[providers]\nlive = {{ kind = \"openai\", base_url = \"{base_url}\" }}\n\
         [models]\nplanner = {{ provider = \"live\", model = \"{model}\", timeout = \"120s\" }}\n"
    );
    let config: ModelsConfig = toml::from_str(&config).unwrap();
    let host = ModelHost::new(config, Secrets::default(), std::env::temp_dir());
    let planner = host.llm(LlmRole::Planner, None).unwrap();
    let started = Instant::now();
    llm_contract(planner.as_ref());
    let contract = started.elapsed();
    let request = LlmRequest {
        system: "Write three short search queries, one per line, about fixing a code signing error."
            .to_owned(),
        messages: vec![LlmMessage::User("Go.".to_owned())],
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: 96,
    };
    let started = Instant::now();
    let mut first_text = None;
    let mut deltas = 0;
    let reply = planner
        .complete(&request, &mut |_| {
            deltas += 1;
            first_text.get_or_insert_with(|| started.elapsed());
        })
        .unwrap();
    let finished = started.elapsed();
    assert!(deltas > 1 && first_text.unwrap() < finished, "a live server streams in several deltas");
    eprintln!(
        "live {base_url}: llm_contract in {contract:?}; streamed {deltas} deltas, first text after {:?} of {finished:?}: {:?}",
        first_text.unwrap(),
        reply.text
    );
}
