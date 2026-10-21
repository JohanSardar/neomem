use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{EpochMs, IntakeId, IntakeRecord, Source, Span};
use neomem_proto::{Request, Response, SearchQuery, ShownMemory};
use neomem_testkit::{FakeRules, LlmRule};
use rusqlite::{Connection, OpenFlags};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;

const EXTRACTOR: &str = "You are neomem's memory extractor";
const WAIT: Duration = Duration::from_secs(10);

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    seed::copy_vault_fixture("edge-ship-date", neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 2, 2)?;
    let turn = add(
        &neomem,
        "claude-code",
        Some("session-1"),
        "Johan: the Edge beta opens next Thursday for 50 testers",
        &["Edge"],
    )?;
    a_talking_session_is_not_extracted_yet(proof, &neomem)?;
    neomem.set_clock(now().plus_millis(6 * 60_000))?;
    let report = support::wait_for_index(&neomem, proof, 3, 2)?;
    proof.claim(
        "after five minutes of silence the session's turn becomes a fact about the existing entity, with no duplicate entity",
        report.is_some_and(|report| report.facts == 3 && report.entities == 2),
        "",
    );
    the_fact_cites_its_turn_and_is_searchable(proof, &neomem, turn)?;
    the_model_never_sees_a_real_id_but_sees_the_dates(proof, &neomem)?;
    a_full_batch_from_another_source_goes_at_once(proof, &neomem)?;
    invented_sources_are_rejected(proof, &neomem)?;
    corrected_records_are_skipped(proof, &neomem)?;
    reextraction_adds_no_duplicates(proof, &neomem)?;
    the_run_log_names_the_model_and_counts(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn now() -> EpochMs {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    EpochMs::from_millis(i64::try_from(since.as_millis()).unwrap_or(0))
}

fn add(
    neomem: &IsolatedNeomem,
    source: &str,
    session: Option<&str>,
    content: &str,
    hints: &[&str],
) -> anyhow::Result<IntakeId> {
    let record = IntakeRecord {
        source: Source::new(source)?,
        occurred: Span::instant(now()),
        tags: Vec::new(),
        content: content.to_owned(),
        entity_hints: hints.iter().map(ToString::to_string).collect(),
        source_ref: None,
        session: session.map(str::to_owned),
    };
    match neomem.client()?.call(Request::Add(record))? {
        Response::Added(receipt) => Ok(receipt.id),
        other => anyhow::bail!("add answered {other:?}"),
    }
}

fn facts_count(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<usize> {
    Ok(support::index_report(neomem, proof)?.map_or(0, |report| report.facts))
}

fn a_talking_session_is_not_extracted_yet(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    thread::sleep(Duration::from_millis(1500));
    let facts = facts_count(neomem, proof)?;
    proof.claim("a turn in a session that is still talking is not extracted yet", facts == 2, facts);
    Ok(())
}

fn the_fact_cites_its_turn_and_is_searchable(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    turn: IntakeId,
) -> anyhow::Result<()> {
    let hits = match neomem.call(
        proof,
        Request::Search(SearchQuery {
            text: "when does the Edge beta open".to_owned(),
            ..SearchQuery::default()
        }),
    )? {
        Response::SearchResults(results) => results.hits,
        _ => Vec::new(),
    };
    let found = hits.iter().find(|hit| hit.fact.content().contains("beta"));
    proof.claim(
        "the new fact is searchable at once and cites the exact turn it came from",
        found.is_some_and(|hit| {
            hit.fact.sources() == [turn] && hit.entities.iter().any(|entity| entity.name == "Edge")
        }),
        found.map_or_else(|| "not found".to_owned(), |hit| hit.fact.content().to_owned()),
    );
    let shown: Option<ShownMemory> = match found.map(|hit| hit.fact.id()) {
        Some(id) => match neomem.call(proof, Request::Show { id: id.into() })? {
            Response::Shown(shown) => Some(*shown),
            _ => None,
        },
        None => None,
    };
    proof.claim(
        "its Markdown file exists in the vault",
        shown.is_some_and(|shown| shown.text.contains("beta")),
        "",
    );
    Ok(())
}

fn the_model_never_sees_a_real_id_but_sees_the_dates(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let calls = neomem.fake_model_calls()?;
    let prompts: Vec<String> = calls
        .iter()
        .filter(|call| call.route == "/v1/chat/completions" && call.body.to_string().contains(EXTRACTOR))
        .map(|call| call.body.to_string())
        .collect();
    let leaked =
        prompts.iter().any(|prompt| ["int_", "fct_", "ent_"].iter().any(|prefix| prompt.contains(prefix)));
    proof.claim(
        "the extraction prompt never shows a real ID",
        !prompts.is_empty() && !leaked,
        format!("{} prompts", prompts.len()),
    );
    let dated = prompts.iter().any(|prompt| {
        prompt.contains("Today is ") && prompt.contains("Known entities") && prompt.contains("Edge")
    });
    proof.claim("it gives today's date, each record's time and the known entities", dated, "");
    Ok(())
}

fn wait_for_facts(neomem: &IsolatedNeomem, proof: &mut ProofRun, at_least: usize) -> anyhow::Result<usize> {
    let deadline = Instant::now() + WAIT;
    loop {
        let facts = facts_count(neomem, proof)?;
        if facts >= at_least || Instant::now() > deadline {
            return Ok(facts);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn a_full_batch_from_another_source_goes_at_once(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let before = facts_count(neomem, proof)?;
    for day in 0..20 {
        add(
            neomem,
            "oura-digest",
            None,
            &format!("Oura sleep report {day}: Johan slept {} hours", 6 + day % 3),
            &["Johan"],
        )?;
    }
    let after = wait_for_facts(neomem, proof, before + 20)?;
    proof.claim(
        "20 records from one source are extracted at once, without waiting for the 30-minute timer",
        after == before + 20,
        format!("{before} -> {after}"),
    );
    Ok(())
}

fn invented_sources_are_rejected(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let invented = r#"{"facts":[{"content":"Johan owns a red bicycle.","sources":[7],"about":["E1"],"valid_from":null,"importance":3,"durability":"ordinary","tags":[]}],"entities":[{"ref":"E1","name":"Johan","kind":"person","aliases":[],"existing":null}]}"#;
    neomem.add_fake_rules(&FakeRules {
        llm: vec![LlmRule {
            system_contains: Some(EXTRACTOR.to_owned()),
            user_contains: Some("red bicycle".to_owned()),
            reply: invented.to_owned(),
            ..LlmRule::default()
        }],
        ..FakeRules::default()
    })?;
    let before = facts_count(neomem, proof)?;
    add(neomem, "note", Some("bike"), "Johan mentioned a red bicycle", &[])?;
    neomem.set_clock(now().plus_millis(20 * 60_000))?;
    thread::sleep(Duration::from_secs(3));
    let after = facts_count(neomem, proof)?;
    proof.claim(
        "a fact citing a record number that does not exist is rejected",
        after == before,
        format!("{before} -> {after}"),
    );
    Ok(())
}

fn corrected_records_are_skipped(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let wrong =
        add(neomem, "note", Some("wrong-night"), "Johan slept 2 hours on the wrong night", &["Johan"])?;
    neomem.client()?.call(Request::Correct { intake: wrong, reason: "wrong night".to_owned() })?;
    let before = facts_count(neomem, proof)?;
    neomem.set_clock(now().plus_millis(40 * 60_000))?;
    thread::sleep(Duration::from_secs(3));
    let after = facts_count(neomem, proof)?;
    proof.claim(
        "a record corrected before extraction never becomes a fact",
        after == before,
        format!("{before} -> {after}"),
    );
    Ok(())
}

fn reextraction_adds_no_duplicates(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let before = facts_count(neomem, proof)?;
    let output = neomem.neomem(&["reextract", "--source", "oura-digest"])?;
    let printed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    neomem.set_clock(now().plus_millis(80 * 60_000))?;
    thread::sleep(Duration::from_secs(4));
    let after = facts_count(neomem, proof)?;
    proof.claim(
        "neomem reextract re-runs extraction from the intake log, and identical facts are recognised instead of duplicated",
        output.status.success() && printed.contains("queued 20") && after == before,
        format!("{printed}; facts {before} -> {after}"),
    );
    Ok(())
}

fn the_run_log_names_the_model_and_counts(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let state = Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement =
        state.prepare("SELECT ifnull(model, ''), changes FROM runs WHERE kind = 'extract' ORDER BY id")?;
    let runs: Vec<(String, String)> =
        statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
    let rejected = runs.iter().any(|(_, changes)| changes.contains("\"rejected\":1"));
    let named = runs.iter().all(|(model, changes)| !model.is_empty() && !changes.contains("Johan"));
    proof.claim(
        "every extraction run is logged with its model and counts, never content, and the rejection is counted",
        !runs.is_empty() && named && rejected,
        format!("{} runs", runs.len()),
    );
    Ok(())
}
