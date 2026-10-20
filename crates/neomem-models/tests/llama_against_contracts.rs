#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use neomem_core::{Embedder, Purpose};
use neomem_models::LlamaEmbedder;
use neomem_testkit::embedder_contract;

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
fn a_missing_model_file_is_reported_not_loaded() {
    let error = LlamaEmbedder::open(&shared_model("no-such-model.gguf")).unwrap_err();
    assert!(error.to_string().contains("missing"));
}
