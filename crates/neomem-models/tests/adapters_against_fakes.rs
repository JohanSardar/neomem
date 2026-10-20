#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use neomem_core::Judge;
use neomem_models::{JevJudge, OpenAiEmbedder};
use neomem_testkit::{FakeModelServer, embedder_contract, judge_contract};

fn fake_base() -> String {
    let address = FakeModelServer::bind(0).unwrap().serve_in_background().unwrap();
    format!("http://{address}")
}

#[test]
fn the_openai_embeddings_adapter_passes_the_embedder_contract() {
    let embedder = OpenAiEmbedder::connect(&format!("{}/v1", fake_base()), None, "hash-256").unwrap();
    embedder_contract(&embedder);
}

#[test]
fn the_jev_adapter_passes_the_judge_contract() {
    let judge = JevJudge::new(Some(&fake_base()), "fake-key".to_owned(), "jev-fake", Duration::from_secs(2));
    judge_contract(&judge);
}

#[test]
fn the_jev_adapter_maps_choice_probabilities_back_to_option_order() {
    let judge = JevJudge::new(Some(&fake_base()), "fake-key".to_owned(), "jev-fake", Duration::from_secs(2));
    let options = [
        neomem_core::Choice { label: "new", meaning: "unrelated" },
        neomem_core::Choice { label: "same", meaning: "same" },
    ];
    let decision = judge.decide("context", "relation?", &options).unwrap();
    assert_eq!(decision.choice(), 0);
}
