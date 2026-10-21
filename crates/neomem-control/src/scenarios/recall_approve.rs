use super::recall_support::{self as support, NEW_DATE, OLD_DATE, PROMPT, SIGNING};
use crate::binaries::Binaries;
use crate::proof::ProofRun;
use neomem_testkit::FakeRules;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = support::setup(proof, binaries)?;
    neomem.add_fake_rules(&FakeRules {
        judge: vec![
            support::rule("this memory", None, 0.69),
            support::rule("this memory", Some("code signing error was fixed"), 0.7),
        ],
        ..FakeRules::default()
    })?;
    let before = neomem.fake_model_calls()?.len();
    let (memories, elapsed) = support::recall(&neomem, proof, "approval", PROMPT)?;
    let calls = neomem.fake_model_calls()?;
    proof.claim(
        "one approval call keeps P >= 0.7 and drops P < 0.7",
        support::judge_calls(&calls[before..]) == 2
            && calls[before..]
                .iter()
                .rfind(|call| call.route == "/v1/systemone")
                .and_then(|call| call.body["questions"].as_object())
                .is_some_and(|questions| questions.len() <= 15)
            && memories.len() == 1
            && memories[0].id.to_string() == SIGNING,
        format!("{} memories; {:.1} ms", memories.len(), elapsed.as_secs_f64() * 1000.0),
    );
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let (repeated, _) = support::recall(&neomem, proof, "approval", PROMPT)?;
    proof.claim(
        "a fact is never injected twice in the same session, even after restarting",
        repeated.is_empty(),
        repeated.len(),
    );
    neomem.add_fake_rules(&FakeRules {
        judge: vec![support::rule("this memory", None, 0.9)],
        ..FakeRules::default()
    })?;
    let (many, _) = support::recall(&neomem, proof, "cap", "Edge ship date Sam Acme Maya dark mode")?;
    proof.claim("at most five approved facts are injected", many.len() == 5, many.len());
    neomem.add_fake_rules(&FakeRules {
        judge: vec![
            support::rule("this memory", None, 0.1),
            support::rule("this memory", Some("ship date"), 0.9),
        ],
        ..FakeRules::default()
    })?;
    let (latest, _) = support::recall(&neomem, proof, "latest", "Edge ship date")?;
    proof.claim(
        "a replaced fact is never injected",
        latest.iter().any(|memory| memory.id.to_string() == NEW_DATE)
            && latest.iter().all(|memory| memory.id.to_string() != OLD_DATE),
        latest.len(),
    );
    let lock = rusqlite::Connection::open(neomem.home().state_db_path())?;
    lock.execute_batch("BEGIN IMMEDIATE")?;
    let (locked, elapsed) = support::recall(&neomem, proof, "locked", PROMPT)?;
    lock.execute_batch("ROLLBACK")?;
    proof.claim(
        "a locked state writer fails open without waiting past the deadline",
        locked.is_empty() && elapsed.as_millis() < 500,
        format!("{:.1} ms", elapsed.as_secs_f64() * 1000.0),
    );
    let (unlocked, _) = support::recall(&neomem, proof, "locked", PROMPT)?;
    proof.claim(
        "a failed state transaction does not mark any fact as injected",
        !unlocked.is_empty(),
        unlocked.len(),
    );
    log_contains_only_metadata(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn log_contains_only_metadata(
    proof: &mut ProofRun,
    neomem: &crate::isolated::IsolatedNeomem,
) -> anyhow::Result<()> {
    let connection = support::db(neomem)?;
    let mut statement = connection.prepare("SELECT candidates, injected, timings, models FROM recall_log")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut holds = true;
    let mut boundary_score = false;
    let mut count = 0;
    for row in rows {
        let (candidates, injected, timings, models) = row?;
        let candidates: Vec<(neomem_core::FactId, Option<f32>)> = serde_json::from_str(&candidates)?;
        let injected: Vec<neomem_core::FactId> = serde_json::from_str(&injected)?;
        let timings: Vec<(String, f64)> = serde_json::from_str(&timings)?;
        let models: Vec<String> = serde_json::from_str(&models)?;
        boundary_score |= candidates.iter().any(|(id, score)| {
            id.to_string() == SIGNING && score.is_some_and(|score| (score - 0.7).abs() < f32::EPSILON)
        });
        holds &= candidates.len() <= 15 && injected.len() <= 5;
        holds &= candidates.iter().all(|(_, score)| score.is_none_or(|score| (0.0..=1.0).contains(&score)));
        holds &= timings.iter().all(|(name, value)| {
            ["gate_ms", "first_search_ms", "search_ms", "plan_ms", "approval_ms", "total_ms", "timeout_ms"]
                .contains(&name.as_str())
                && value.is_finite()
                && *value >= 0.0
        });
        holds &= !models.is_empty()
            && models.iter().all(|model| !model.contains(PROMPT) && !model.contains("signing error"));
        count += 1;
    }
    proof.claim(
        "recall log stores IDs, scores, counts, model IDs and timings without content",
        holds && boundary_score && count > 0,
        format!("{count} typed metadata records; boundary score found: {boundary_score}"),
    );
    Ok(())
}
