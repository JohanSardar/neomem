#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use neomem_core::{Choice, Embedder, Judge, Purpose};
use neomem_models::{LlamaEmbedder, LlamaReranker};
use neomem_testkit::{embedder_contract, judge_contract};

fn shared_model(file: &str) -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    let common = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    common.parent().unwrap().join("models").join(file)
}

fn query_beats_unrelated(embedder: &dyn Embedder) -> bool {
    let query = &embedder.embed(&["when does Edge ship"], Purpose::Query).unwrap()[0];
    let documents =
        embedder.embed(&["Edge ships on November 1", "Maya is my sister"], Purpose::Document).unwrap();
    query.cosine(&documents[0]) > query.cosine(&documents[1]) + 0.1
}

#[test]
#[ignore = "loads models/embeddinggemma-300m-Q8_0.gguf; run after neomemctl fetch-models"]
fn embeddinggemma_passes_the_embedder_contract() {
    let embedder = LlamaEmbedder::open(&shared_model("embeddinggemma-300m-Q8_0.gguf")).unwrap();
    embedder_contract(&embedder);
    assert_eq!(embedder.dims(), 768);
    assert!(query_beats_unrelated(&embedder));
}

#[test]
#[ignore = "loads models/qwen3-embedding-0.6b-Q8_0.gguf; run after neomemctl fetch-models"]
fn qwen3_embedding_passes_the_embedder_contract() {
    let embedder = LlamaEmbedder::open(&shared_model("qwen3-embedding-0.6b-Q8_0.gguf")).unwrap();
    embedder_contract(&embedder);
    assert_eq!(embedder.dims(), 1024);
    assert!(query_beats_unrelated(&embedder));
}

#[test]
#[ignore = "loads models/qwen3-reranker-0.6b-Q8_0.gguf; run after neomemctl fetch-models"]
fn qwen3_reranker_passes_the_judge_contract_and_ranks_what_helps_first() {
    let reranker = LlamaReranker::open(&shared_model("qwen3-reranker-0.6b-Q8_0.gguf")).unwrap();
    judge_contract(&reranker);
    let scores = reranker
        .score(
            "The release build failed again with the same code signing error as last week",
            "Would this memory help with what is happening right now?",
            &[
                "Signing the Edge release needs the certificate from the team keychain",
                "Maya likes green tea",
            ],
        )
        .unwrap();
    assert!(scores[0] > scores[1], "{scores:?}");
    let alone = reranker
        .score(
            "The release build failed again with the same code signing error as last week",
            "Would this memory help with what is happening right now?",
            &["Signing the Edge release needs the certificate from the team keychain"],
        )
        .unwrap();
    assert!((alone[0] - scores[0]).abs() < 0.01, "batched {scores:?} alone {alone:?}");
    let options = [
        Choice { label: "updates", meaning: "the new fact replaces the existing fact" },
        Choice { label: "new", meaning: "the new fact is about something else" },
    ];
    let decision = reranker
        .decide(
            "New fact: Edge ships Nov 1. Existing fact: Edge ships Oct 15.",
            "How do the facts relate?",
            &options,
        )
        .unwrap();
    assert!((decision.probabilities.iter().sum::<f32>() - 1.0).abs() < 1e-4);
}

#[test]
fn a_missing_model_file_is_reported_not_loaded() {
    let error = LlamaEmbedder::open(&shared_model("no-such-model.gguf")).unwrap_err();
    assert!(error.to_string().contains("missing"));
}
