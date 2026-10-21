#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{Clock, EpochMs, LlmMessage, LlmRequest, LlmRole, ModelId, ModelRole, ModelRoles};
use neomem_models::{ReloadingHost, Secrets};
use neomem_testkit::{FakeModelServer, FakeRules, RecordedCall};

#[derive(Debug)]
struct FrozenClock;

impl Clock for FrozenClock {
    fn now(&self) -> EpochMs {
        EpochMs::from_millis(0)
    }
}

struct World {
    directory: tempfile::TempDir,
    address: String,
}

impl World {
    fn new() -> Self {
        let address = FakeModelServer::bind(0).unwrap().serve_in_background().unwrap().to_string();
        let directory = tempfile::tempdir().unwrap();
        let config = format!(
            r#"
[providers]
fake = {{ kind = "openai", base_url = "http://{address}/v1" }}
[models]
embedder = {{ provider = "fake", model = "hash-256" }}
planner = {{ provider = "fake", model = "old-planner" }}
"#
        );
        fs::write(directory.path().join("config.toml"), config).unwrap();
        Self { directory, address }
    }

    fn open(&self) -> ReloadingHost {
        ReloadingHost::open(
            self.directory.path().join("config.toml"),
            self.directory.path().join("models"),
            Secrets::default(),
            Arc::new(FrozenClock),
        )
        .unwrap()
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
        system: "Say OK.".to_owned(),
        messages: vec![LlmMessage::User("ping".to_owned())],
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: 16,
    }
}

#[test]
fn historical_embedding_configuration_survives_a_restart() {
    let world = World::new();
    let host = world.open();
    let old = host.embedder().unwrap();
    host.use_model(ModelRole::Embedder, "fake/hash-384").unwrap();
    let new = host.embedder().unwrap();
    assert_eq!(new.dims(), 384);
    host.retain_embedders(&[old.model_id()]);
    assert!(Arc::ptr_eq(&new, &host.embedder_for(&new.model_id()).unwrap()));
    host.retain_embedders(&[new.model_id()]);
    assert_eq!(Arc::strong_count(&old), 1);
    assert_eq!(host.embedder_for(&old.model_id()).unwrap().dims(), 256);
    drop(host);
    let restarted = world.open();
    assert_eq!(restarted.embedder().unwrap().dims(), 384);
    assert_eq!(restarted.embedder_for(&ModelId::new("openai:hash-256")).unwrap().dims(), 256);
}

#[test]
fn role_reload_changes_the_wire_request_and_keeps_an_unchanged_embedder() {
    let world = World::new();
    let host = world.open();
    let before = host.embedder().unwrap();
    host.use_model(ModelRole::Planner, "new-planner").unwrap();
    host.llm(LlmRole::Planner, None).unwrap().complete(&ping(), &mut |_| {}).unwrap();
    assert!(world.calls().iter().any(|call| call.body["model"] == "new-planner"));
    assert!(Arc::ptr_eq(&before, &host.embedder_for(&before.model_id()).unwrap()));
    let config = world.directory.path().join("config.toml");
    let edited = fs::read_to_string(&config).unwrap().replace("new-planner", "edited-planner");
    fs::write(config, edited).unwrap();
    host.llm(LlmRole::Planner, None).unwrap().complete(&ping(), &mut |_| {}).unwrap();
    assert!(world.calls().iter().any(|call| call.body["model"] == "edited-planner"));
    assert!(Arc::ptr_eq(&before, &host.embedder_for(&before.model_id()).unwrap()));
}

#[test]
fn preparing_a_new_model_does_not_lock_out_the_active_model() {
    let world = World::new();
    let host = Arc::new(world.open());
    let old = host.embedder().unwrap();
    let rules =
        FakeRules { latency_ms: [("/v1/embeddings".to_owned(), 2000)].into(), ..FakeRules::default() };
    ureq::post(&format!("http://{}/fake/rules", world.address)).send_json(&rules).unwrap();
    let swapping = Arc::clone(&host);
    let worker = thread::spawn(move || swapping.use_model(ModelRole::Embedder, "fake/hash-384"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !world.calls().iter().any(|call| call.body["model"] == "hash-384") {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    assert_eq!(host.embedder_for(&old.model_id()).unwrap().dims(), 256);
    assert!(started.elapsed() < Duration::from_millis(250));
    worker.join().unwrap().unwrap();
}
