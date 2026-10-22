use std::path::Path;
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
    claim_prepared(proof, binaries, &directory, &settings)?;
    claim_rescore(proof, binaries, &directory, &settings, &items)
}

fn claim_rescore(
    proof: &mut ProofRun,
    binaries: &Binaries,
    directory: &Path,
    settings: &Path,
    items: &[Value],
) -> anyhow::Result<()> {
    let rescored = directory.with_file_name("rescored");
    let output = Command::new(binaries.neomemd.with_file_name("neomem-eval"))
        .arg("rescore")
        .arg(directory)
        .arg("--dataset")
        .arg(seed::fixture_dir("eval")?.join("questions.json"))
        .arg("--models")
        .arg(settings)
        .arg("--out")
        .arg(&rescored)
        .env("NEOMEM_FAKE_OPENAI_KEY", "fake-key")
        .output()?;
    let again: Vec<Value> = std::fs::read(rescored.join("items.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let same_hits = again.len() == items.len()
        && again
            .iter()
            .zip(items)
            .all(|(new, old)| new["id"] == old["id"] && new["recall5"] == old["recall5"]);
    proof.claim(
        "rescoring a run answers every question again from its saved search hits",
        output.status.success() && same_hits,
        format!(
            "{} of {} questions; {}",
            again.len(),
            items.len(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    );
    Ok(())
}

fn claim_prepared(
    proof: &mut ProofRun,
    binaries: &Binaries,
    directory: &Path,
    settings: &Path,
) -> anyhow::Result<()> {
    let binary = binaries.neomemd.with_file_name("neomem-eval");
    let prepared = directory.with_file_name("prepared");
    let fixture = seed::fixture_dir("eval")?.join("questions.json");
    let status = Command::new(&binary)
        .arg("prepare")
        .arg(&fixture)
        .args(["--sample", "3", "--shards", "2", "--out"])
        .arg(&prepared)
        .output()?;
    anyhow::ensure!(status.status.success(), "prepare failed: {}", String::from_utf8_lossy(&status.stderr));
    let mut matches = true;
    for shard in 0..2 {
        let direct = directory.with_file_name(format!("direct-{shard}"));
        let compact = directory.with_file_name(format!("compact-{shard}"));
        for (dataset, out, flags) in [
            (
                fixture.clone(),
                &direct,
                vec![
                    "--sample".to_owned(),
                    "3".to_owned(),
                    "--shard".to_owned(),
                    shard.to_string(),
                    "--shards".to_owned(),
                    "2".to_owned(),
                ],
            ),
            (prepared.join(format!("shard-{shard}.json")), &compact, vec!["--prepared".to_owned()]),
        ] {
            let output = Command::new(&binary)
                .arg("run")
                .arg(dataset)
                .args(flags)
                .arg("--models")
                .arg(settings)
                .arg("--out")
                .arg(out)
                .args(["--arm", "memory-off"])
                .env("NEOMEM_FAKE_OPENAI_KEY", "fake-key")
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "shard failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let read = |path: &Path| -> anyhow::Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(path.join("manifest.json"))?)?)
        };
        let first = read(&direct)?;
        let second = read(&compact)?;
        matches &= ["selected", "held_out", "dataset_sha256", "seed", "split", "shard", "shards"]
            .iter()
            .all(|key| first[key] == second[key]);
        let answered = |path: &Path| -> anyhow::Result<Vec<Value>> {
            let items: Vec<Value> = serde_json::from_slice(&std::fs::read(path.join("items.json"))?)?;
            Ok(items.into_iter().map(|item| item["id"].clone()).collect())
        };
        matches &= answered(&direct)? == answered(&compact)?;
    }
    proof.claim(
        "prepared shard runs answer the same questions with the original dataset identity",
        matches,
        "two shards; manifests and answered ids match",
    );
    Ok(())
}
