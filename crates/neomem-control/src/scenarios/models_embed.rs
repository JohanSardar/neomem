use std::time::Duration;

use neomem_core::ModelRole;
use neomem_proto::{ModelTestReport, Request, Response};

use crate::binaries::Binaries;
use crate::isolated::{IsolatedNeomem, Models};
use crate::model_files;
use crate::proof::ProofRun;
use crate::timing;

const QUERY: &str = "when does Edge ship";
const RELATED: &str = "Edge ships on November 1";
const UNRELATED: &str = "Maya is my sister";
const SIMILARITY_MARGIN: f32 = 0.1;
const SAME_SCORE_TOLERANCE: f32 = 1e-5;
const QUERY_LATENCY_SANITY_P50_MS: f64 = 150.0;
const CACHED_QUERY_CEILING_MS: f64 = 1.0;

const QUERY_TOPICS: &[&str] = &[
    "what did we decide about the signing certificate for the Edge release build",
    "how do I fix the flaky integration test that times out on the macOS runner",
    "which database migration broke the nightly backup job and who owns the rollback",
    "when is Maya's next dentist appointment and did we already pay the last invoice",
    "what is the default timeout for the judge role and where is it set in the config",
    "which embedding model did we pick for neomem and why did it beat the other one",
];

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    "real llama.cpp embedder, fake OpenAI-compatible embedder".clone_into(&mut proof.models);
    let mut real = IsolatedNeomem::start_with(binaries, Models::Real)?;
    the_local_embedder_answers_the_cli(proof, &real)?;
    related_text_scores_higher(proof, &real)?;
    the_same_inputs_embed_the_same(proof, &mut real)?;
    let uncached_p50 = query_latency_is_recorded(proof, &real)?;
    a_repeated_query_skips_the_model(proof, &real, uncached_p50)?;
    proof.attach_logs(&real.logs());
    let fake = IsolatedNeomem::start(binaries)?;
    an_openai_compatible_endpoint_answers(proof, &fake)?;
    the_cache_wraps_any_embedder(proof, &fake)?;
    proof.attach_logs(&fake.logs());
    Ok(())
}

fn test_embedder(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    inputs: &[&str],
) -> anyhow::Result<Result<ModelTestReport, String>> {
    let request = Request::ModelsTest {
        role: ModelRole::Embedder,
        source: None,
        structured: false,
        inputs: inputs.iter().map(ToString::to_string).collect(),
    };
    Ok(match neomem.call(proof, request)? {
        Response::ModelTested(report) => Ok(report),
        other => Err(format!("{other:?}")),
    })
}

fn describe(outcome: &Result<ModelTestReport, String>) -> String {
    match outcome {
        Ok(report) => format!(
            "{} answered {:?} in {:.2} ms, scores {:?}",
            report.model, report.answer, report.latency_ms, report.scores
        ),
        Err(problem) => problem.clone(),
    }
}

fn local_model_id() -> String {
    format!("llama.cpp:{}", model_files::EMBEDDINGGEMMA.name.trim_end_matches(".gguf"))
}

fn the_local_embedder_answers_the_cli(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let output = neomem.neomem(&["models", "test", "embedder", "--json"])?;
    let parsed: Option<Response> = serde_json::from_slice(&output.stdout).ok();
    let holds = output.status.success()
        && matches!(&parsed, Some(Response::ModelTested(report))
            if report.model == local_model_id() && report.answer == "768 dimensions");
    proof.claim(
        "neomem models test embedder answers from EmbeddingGemma-300m in-process through llama.cpp with 768 dimensions",
        holds,
        String::from_utf8_lossy(&output.stdout).trim(),
    );
    Ok(())
}

fn related_text_scores_higher(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let outcome = test_embedder(proof, neomem, &[QUERY, RELATED, UNRELATED])?;
    let holds = matches!(&outcome, Ok(report)
        if report.scores.len() == 2 && report.scores[0] > report.scores[1] + SIMILARITY_MARGIN);
    proof.claim(
        "a query scores a related fact clearly higher than an unrelated one",
        holds,
        describe(&outcome),
    );
    Ok(())
}

fn the_same_inputs_embed_the_same(proof: &mut ProofRun, neomem: &mut IsolatedNeomem) -> anyhow::Result<()> {
    let inputs = [QUERY, RELATED, UNRELATED];
    let first = test_embedder(proof, neomem, &inputs)?;
    let again = test_embedder(proof, neomem, &inputs)?;
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let restarted = test_embedder(proof, neomem, &inputs)?;
    let scores = |outcome: &Result<ModelTestReport, String>| {
        outcome.as_ref().map(|report| report.scores.clone()).unwrap_or_default()
    };
    let same = |left: &[f32], right: &[f32]| {
        left.len() == 2
            && left.len() == right.len()
            && left.iter().zip(right).all(|(one, other)| (one - other).abs() <= SAME_SCORE_TOLERANCE)
    };
    let (first, again, restarted) = (scores(&first), scores(&again), scores(&restarted));
    proof.claim(
        "the same texts embed the same across calls and across a daemon restart",
        same(&first, &again) && same(&first, &restarted),
        format!("{first:?} / {again:?} / after restart {restarted:?}"),
    );
    Ok(())
}

fn distinct_queries() -> Vec<String> {
    let words: Vec<&str> = QUERY_TOPICS.iter().flat_map(|topic| topic.split_whitespace()).collect();
    (0..30)
        .map(|round| {
            let length = 13 + round % 12;
            let start = (round * 7) % (words.len() - length);
            format!("{} {round}", words[start..start + length].join(" "))
        })
        .collect()
}

fn query_latency_is_recorded(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<f64> {
    let _warm_up = test_embedder(proof, neomem, &["warm up the model"])?;
    let mut samples = Vec::new();
    let mut failures = Vec::new();
    for query in distinct_queries() {
        match test_embedder(proof, neomem, &[&query])? {
            Ok(report) => samples.push(Duration::from_secs_f64(report.latency_ms / 1000.0)),
            Err(problem) => failures.push(problem),
        }
    }
    let latency = timing::percentiles(&mut samples);
    proof.claim(
        "30 distinct 16-32 token queries embed in-process (p50 and p95 recorded; §13.5 budgets 5-20 ms on a release build)",
        failures.is_empty() && samples.len() == 30 && latency.p50_ms < QUERY_LATENCY_SANITY_P50_MS,
        format!("{latency}; {} failures, first {:?}", failures.len(), failures.first()),
    );
    Ok(latency.p50_ms)
}

fn a_repeated_query_skips_the_model(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    uncached_p50: f64,
) -> anyhow::Result<()> {
    let query = distinct_queries().remove(0);
    let repeated = test_embedder(proof, neomem, &[&query])?;
    let holds = matches!(&repeated, Ok(report)
        if report.latency_ms < CACHED_QUERY_CEILING_MS && report.latency_ms * 10.0 < uncached_p50);
    proof.claim(
        "a repeated query is answered from the query cache, far faster than the model",
        holds,
        format!("{}; uncached p50 {uncached_p50:.2} ms", describe(&repeated)),
    );
    Ok(())
}

fn an_openai_compatible_endpoint_answers(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let outcome = test_embedder(proof, neomem, &[QUERY, RELATED, UNRELATED])?;
    let holds = matches!(&outcome, Ok(report)
        if report.model == "openai:hash-256" && report.answer == "256 dimensions" && report.scores.len() == 2);
    proof.claim(
        "an OpenAI-compatible embeddings endpoint answers through the same command",
        holds,
        describe(&outcome),
    );
    Ok(())
}

fn the_cache_wraps_any_embedder(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let query = "which certificate signs the Edge build";
    let _first = test_embedder(proof, neomem, &[query])?;
    let repeated = test_embedder(proof, neomem, &[query])?;
    let reached_the_endpoint = neomem
        .fake_model_calls()?
        .iter()
        .filter(|call| call.route == "/v1/embeddings" && call.body["input"].to_string().contains(query))
        .count();
    proof.claim(
        "the query cache wraps any embedder: a repeated query reaches the OpenAI-compatible endpoint once",
        reached_the_endpoint == 1 && repeated.is_ok(),
        format!("{reached_the_endpoint} endpoint calls; {}", describe(&repeated)),
    );
    Ok(())
}
