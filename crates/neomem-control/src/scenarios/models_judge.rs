use std::collections::BTreeMap;
use std::time::Duration;

use neomem_core::ModelRole;
use neomem_proto::{ModelTestReport, Request, Response};
use neomem_testkit::{FakeRules, JudgeAnswer, JudgeRule};

use crate::binaries::Binaries;
use crate::isolated::{IsolatedNeomem, Models};
use crate::model_files;
use crate::proof::ProofRun;
use crate::repo;
use crate::timing;

const SYSTEM_ONE: &str = "/v1/systemone";
const SLOW_JEV_MS: u64 = 2000;
const JEV_TIMEOUT_MS: f64 = 600.0;
const FALLBACK_SANITY_MS: f64 = 1500.0;
const LOCAL_SANITY_MS: f64 = 600.0;
const FELL_BACK: &str = "the local fallback answers this call";
const SITUATION: &str = "The release build failed again with the same code signing error as last week";
const HELPS: &str = "Signing the Edge release needs the certificate from the team keychain";
const UNRELATED: &str = "Maya likes green tea in the afternoon";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    "fake Jev with the real llama.cpp reranker as fallback, live Jev".clone_into(&mut proof.models);
    model_files::ensure(&repo::shared_models_dir(&repo::root()?), &[model_files::QWEN3_RERANKER])?;
    let mut fake = IsolatedNeomem::start(binaries)?;
    jev_answers_the_judge(proof, &fake)?;
    a_slow_jev_hands_the_call_to_the_reranker(proof, &fake)?;
    without_a_key_the_reranker_answers_alone(proof, &mut fake)?;
    the_reranker_ranks_what_helps_first(proof, &fake)?;
    proof.attach_logs(&fake.logs());
    let live = IsolatedNeomem::start_with(binaries, Models::Real)?;
    if live.has_live_key("JEV_API_KEY") {
        the_live_jev_api_answers(proof, &live)?;
    } else {
        eprintln!("models.judge: no JEV_API_KEY in ~/.neomem/secrets.env; the live Jev claim needs the user");
    }
    proof.attach_logs(&live.logs());
    Ok(())
}

fn test_judge(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    inputs: &[&str],
) -> anyhow::Result<Result<ModelTestReport, String>> {
    let request = Request::ModelsTest {
        role: ModelRole::Judge,
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
            "{} answered {:?} in {:.1} ms, scores {:?}",
            report.model, report.answer, report.latency_ms, report.scores
        ),
        Err(problem) => problem.clone(),
    }
}

fn reranker_id() -> String {
    format!("llama.cpp:{}", model_files::QWEN3_RERANKER.name.trim_end_matches(".gguf"))
}

fn jev_calls(neomem: &IsolatedNeomem) -> anyhow::Result<usize> {
    Ok(neomem.fake_model_calls()?.iter().filter(|call| call.route == SYSTEM_ONE).count())
}

fn fallbacks_logged(neomem: &IsolatedNeomem) -> usize {
    neomem.logs().matches(FELL_BACK).count()
}

fn timed(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    calls: usize,
) -> anyhow::Result<(Vec<Result<ModelTestReport, String>>, timing::Percentiles)> {
    let outcomes: Vec<_> = (0..calls).map(|_| test_judge(proof, neomem, &[])).collect::<Result<_, _>>()?;
    let mut samples: Vec<Duration> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok())
        .map(|report| Duration::from_secs_f64(report.latency_ms / 1000.0))
        .collect();
    Ok((outcomes, timing::percentiles(&mut samples)))
}

fn jev_answers_the_judge(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let scripted = JudgeRule {
        question_contains: Some("test request".to_owned()),
        context_contains: None,
        candidate_contains: None,
        answer: JudgeAnswer::Choice { label: "yes".to_owned(), probability: 0.93 },
    };
    neomem.add_fake_rules(&FakeRules { judge: vec![scripted], ..FakeRules::default() })?;
    let before = jev_calls(neomem)?;
    let outcome = test_judge(proof, neomem, &[])?;
    let reached = jev_calls(neomem)? - before;
    let holds = matches!(&outcome, Ok(report) if report.model == "jev:jev-fake" && report.answer == "yes (0.93)")
        && reached == 1
        && fallbacks_logged(neomem) == 0;
    proof.claim(
        "neomem models test judge answers through the Jev API (the fake) without touching the fallback",
        holds,
        format!("{}; {reached} Jev calls", describe(&outcome)),
    );
    Ok(())
}

fn a_slow_jev_hands_the_call_to_the_reranker(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let slow = FakeRules {
        latency_ms: BTreeMap::from([(SYSTEM_ONE.to_owned(), SLOW_JEV_MS)]),
        ..FakeRules::default()
    };
    neomem.add_fake_rules(&slow)?;
    let loading = test_judge(proof, neomem, &[])?;
    let (outcomes, latency) = timed(proof, neomem, 5)?;
    let all_answered = loading.is_ok() && outcomes.iter().all(Result::is_ok);
    let holds = all_answered
        && fallbacks_logged(neomem) == 6
        && latency.p50_ms > JEV_TIMEOUT_MS
        && latency.p50_ms < FALLBACK_SANITY_MS;
    proof.claim(
        "when Jev answers slower than the 600 ms role timeout, the local reranker answers each call instead, long before the slow Jev would",
        holds,
        format!(
            "Jev delayed {SLOW_JEV_MS} ms; {} fallbacks logged; first call (loads the reranker) {}; then {latency}",
            fallbacks_logged(neomem),
            describe(&loading)
        ),
    );
    Ok(())
}

fn without_a_key_the_reranker_answers_alone(
    proof: &mut ProofRun,
    neomem: &mut IsolatedNeomem,
) -> anyhow::Result<()> {
    neomem.remove_the_fake_jev_key()?;
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let before = jev_calls(neomem)?;
    let loading = test_judge(proof, neomem, &[])?;
    let (outcomes, latency) = timed(proof, neomem, 5)?;
    let reached = jev_calls(neomem)? - before;
    let holds = loading.is_ok()
        && outcomes.iter().all(|outcome| matches!(outcome, Ok(report) if report.model == reranker_id()))
        && reached == 0
        && latency.p50_ms < LOCAL_SANITY_MS;
    proof.claim(
        "with no Jev key the local reranker answers by itself, never calling Jev",
        holds,
        format!("{reached} Jev calls; first call {}; then {latency}", describe(&loading)),
    );
    Ok(())
}

fn the_reranker_ranks_what_helps_first(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let outcome = test_judge(proof, neomem, &[SITUATION, HELPS, UNRELATED])?;
    let holds = matches!(&outcome, Ok(report)
        if report.model == reranker_id() && report.scores.len() == 2 && report.scores[0] > report.scores[1]);
    proof.claim(
        "the local reranker scores the memory that helps right now above an unrelated one",
        holds,
        describe(&outcome),
    );
    Ok(())
}

fn the_live_jev_api_answers(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let (outcomes, latency) = timed(proof, neomem, 5)?;
    let answered = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Ok(report) if report.model == "jev:jev-1.13.0"))
        .count();
    let holds =
        answered == outcomes.len() && fallbacks_logged(neomem) == 0 && latency.p50_ms < JEV_TIMEOUT_MS;
    proof.claim(
        "the live Jev API (jev-1.13.0) answers the judge inside its 600 ms timeout, without the fallback",
        holds,
        format!(
            "{answered}/{} answered; {latency}; first {}",
            outcomes.len(),
            outcomes.first().map(describe).unwrap_or_default()
        ),
    );
    Ok(())
}
