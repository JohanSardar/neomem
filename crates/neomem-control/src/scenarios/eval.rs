use std::process::Command;

use anyhow::Context;
use neomem_testkit::{FakeRules, JudgeAnswer, JudgeRule, LlmRule};
use serde_json::{Value, json};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::{Exchange, ProofRun};
use crate::seed;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let fake = IsolatedNeomem::prepared(binaries)?;
    fake.add_fake_rules(&FakeRules {
        llm: vec![LlmRule {
            system_contains: Some("impartial answer evaluator".to_owned()),
            reply: "yes".to_owned(),
            ..LlmRule::default()
        }],
        judge: vec![JudgeRule {
            question_contains: Some("relate to".to_owned()),
            context_contains: Some("New fact: user: The Osprey bicycle is now blue".to_owned()),
            candidate_contains: None,
            answer: JudgeAnswer::Choice { label: "updates_1".to_owned(), probability: 0.99 },
        }],
        ..FakeRules::default()
    })?;
    let temporary = tempfile::tempdir()?;
    let settings = temporary.path().join("models.json");
    let endpoint = json!({"base_url":format!("http://{}/v1",fake.fake_models_address()?),"model":"fake-reader","version":"testkit","max_tokens":64});
    std::fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "daemon_config":std::fs::read_to_string(fake.home().config_path())?,
            "models_dir":null,"reader":endpoint,"answer_judge":endpoint,"context_budget":10000,"provenance":{"mode":"fake"}
        }))?,
    )?;
    let directory = temporary.path().join("run");
    let output = Command::new(binaries.neomemd.with_file_name("neomem-eval"))
        .arg("run")
        .arg(seed::fixture_dir("eval")?.join("questions.json"))
        .args(["--models"])
        .arg(&settings)
        .arg("--out")
        .arg(&directory)
        .env("NEOMEM_FAKE_OPENAI_KEY", "fake-key")
        .env("NEOMEM_FAKE_JEV_KEY", "fake-key")
        .output()?;
    proof.claim(
        "the fixture runs through isolated daemons over the protocol",
        output.status.success(),
        String::from_utf8_lossy(&output.stderr),
    );
    let items: Vec<Value> = serde_json::from_slice(&std::fs::read(directory.join("items.json"))?)?;
    for exchange in items.iter().flat_map(|item| item["exchanges"].as_array().into_iter().flatten()) {
        proof.record_exchange(Exchange {
            request: exchange["request"].clone(),
            response: exchange["response"].clone(),
            elapsed_ms: exchange["elapsed_ms"].as_f64().unwrap_or_default(),
        });
    }
    let item = |id| items.iter().find(|item| item["id"] == id).context("missing fixture question");
    proof.claim(
        "extracted evidence is a retrieval hit and absent evidence is a miss",
        item("hit")?["recall5"] == 1.0 && item("miss")?["recall5"] == 0.0,
        "source-session coverage",
    );
    let newest = &item("update")?["results"]["hits"][0]["fact"];
    proof.claim(
        "knowledge updates retrieve the newer fact first",
        newest.to_string().contains("blue"),
        newest,
    );
    let isolated = items.iter().all(|item| {
        item["exchanges"].as_array().is_some_and(|exchanges| {
            let has = |command| exchanges.iter().any(|e| e["request"]["command"] == command);
            has("status")
                && has("add")
                && has("search")
                && has("dream")
                && exchanges
                    .iter()
                    .filter_map(|e| e["response"]["body"]["home"].as_str())
                    .all(|home| home.starts_with("/tmp/neomem-eval-"))
        })
    });
    proof.claim(
        "all histories use Add, Search and Dream with temporary homes",
        isolated,
        "protocol exchanges recorded per question",
    );
    let report: Value = serde_json::from_slice(&std::fs::read(directory.join("report.json"))?)?;
    proof.claim(
        "reports per-type metrics, abstention accuracy and model settings",
        report["by_type"]["abstention"]["accuracy"] == 1.0
            && report["by_type"]["abstention"]["retrieval_questions"] == 0
            && report["settings"]["models"]["reader"]["model"] == "fake-reader",
        "report.json",
    );
    let status = Command::new(binaries.neomemd.with_file_name("neomem-eval"))
        .arg("report")
        .arg(&directory)
        .output()?
        .status;
    proof.claim("report can be regenerated from saved results", status.success(), status);
    Ok(())
}
