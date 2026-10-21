use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::{scenarios::support, seed};
use neomem_proto::{RecallMemory, RecallQuery, Request, Response};
use neomem_testkit::{FakeRules, JudgeAnswer, JudgeRule, LlmRule, RecordedCall};
use rusqlite::{Connection, OpenFlags};
use std::time::{Duration, Instant};

pub const PROMPT: &str = "Edge code signing error again";
pub const SIGNING: &str = "fct_01M55FJ500000000000000000D";
pub const OLD_DATE: &str = "fct_01M50ARQ00000000000000000A";
pub const NEW_DATE: &str = "fct_01M5D6RA00000000000000000B";

pub fn setup(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<IsolatedNeomem> {
    let neomem = IsolatedNeomem::start(binaries)?;
    seed::copy_vault_fixture("workspace", neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 13, 7)?;
    neomem.add_fake_rules(&FakeRules {
        judge: vec![rule("long-term memory", None, 0.9), rule("this memory", None, 0.9)],
        llm: vec![planner(0, 0)],
        ..FakeRules::default()
    })?;
    Ok(neomem)
}

pub fn rule(question: &str, candidate: Option<&str>, probability: f32) -> JudgeRule {
    JudgeRule {
        question_contains: Some(question.to_owned()),
        context_contains: None,
        candidate_contains: candidate.map(str::to_owned),
        answer: JudgeAnswer::Probability(probability),
    }
}

pub fn planner(delay_ms: u64, delta_delay_ms: u64) -> LlmRule {
    LlmRule {
        system_contains: Some("fact-shaped".to_owned()),
        user_contains: None,
        reply: "Edge signing\nEdge ship date\nSam Acme designer\n".to_owned(),
        delay_ms,
        delta_delay_ms,
        times: None,
    }
}

pub fn recall(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    session: &str,
    message: &str,
) -> anyhow::Result<(Vec<RecallMemory>, Duration)> {
    let started = Instant::now();
    let response = neomem.call(
        proof,
        Request::Recall(RecallQuery {
            turn: Some(session.to_owned()),
            agent: "codex".to_owned(),
            session: session.to_owned(),
            slice: "Last turn: release build failed".to_owned(),
            message: message.to_owned(),
            error: Some("error: signing Edge".to_owned()),
            project: None,
        }),
    )?;
    match response {
        Response::Recalled(memories) => Ok((memories, started.elapsed())),
        other => anyhow::bail!("recall answered {other:?}"),
    }
}

pub fn judge_calls(calls: &[RecordedCall]) -> usize {
    calls.iter().filter(|call| call.route == "/v1/systemone").count()
}

pub fn db(neomem: &IsolatedNeomem) -> anyhow::Result<Connection> {
    Ok(Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

pub fn last_log(neomem: &IsolatedNeomem) -> anyhow::Result<(Option<f32>, i64, String, String, String)> {
    Ok(db(neomem)?.query_row(
        "SELECT gate, queries, source, candidates, injected FROM recall_log ORDER BY id DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?)
}
