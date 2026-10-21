use super::recall_support::{self as support, PROMPT};
use crate::binaries::Binaries;
use crate::proof::ProofRun;
use neomem_testkit::FakeRules;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = support::setup(proof, binaries)?;
    neomem.add_fake_rules(&FakeRules { llm: vec![support::planner(0, 120)], ..FakeRules::default() })?;
    let before = neomem.fake_model_calls()?.len();
    let (memories, elapsed) = support::recall(&neomem, proof, "stream", PROMPT)?;
    let calls = neomem.fake_model_calls()?;
    let calls = &calls[before..];
    let first_search = calls.iter().find(|call| call.route == "/v1/embeddings");
    let log = support::last_log(&neomem)?;
    let timings: String = support::db(&neomem)?.query_row(
        "SELECT timings FROM recall_log ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let timings: Vec<(String, f64)> = serde_json::from_str(&timings)?;
    let first_ms = timings.iter().find(|(name, _)| name == "first_search_ms").map(|(_, time)| *time);
    proof.claim(
        "streamed lines enter search before the planner finishes",
        !memories.is_empty()
            && first_search.is_some()
            && log.1 == 3
            && log.2 == "planner"
            && first_ms.is_some_and(|time| time < 700.0)
            && elapsed.as_millis() >= 900,
        format!("first search {first_ms:?} ms; total {:.1} ms", elapsed.as_secs_f64() * 1000.0),
    );
    neomem.add_fake_rules(&FakeRules { llm: vec![support::planner(5000, 0)], ..FakeRules::default() })?;
    let (memories, elapsed) = support::recall(&neomem, proof, "fallback", PROMPT)?;
    let log = support::last_log(&neomem)?;
    proof.claim(
        "a slow planner falls back at 2.5 s and recall finishes under 3.5 s",
        !memories.is_empty()
            && log.2 == "fallback"
            && elapsed.as_millis() >= 2500
            && elapsed.as_millis() < 3500,
        format!("{} queries; {:.1} ms", log.1, elapsed.as_secs_f64() * 1000.0),
    );
    proof.attach_logs(&neomem.logs());
    Ok(())
}
