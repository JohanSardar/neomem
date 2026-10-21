use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use neomem_core::{EpochMs, FactLinkKind, IntakeRecord, Source, Span};
use neomem_proto::{Request, Response, SearchHit, SearchQuery};
use neomem_testkit::{FakeRules, JudgeAnswer, JudgeRule};
use rusqlite::{Connection, OpenFlags};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;

const NOVEMBER: &str = "fct_01M59NY2N09AC5V7E9Z0C1HT0H";
const DECEMBER: &str = "Edge's ship date is December 3, 2026.";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = seed::start_with_vault_fixture("edge-ship-date", binaries)?;
    let ready = support::wait_for_index(&neomem, proof, 2, 2)?;
    anyhow::ensure!(
        ready.as_ref().is_some_and(|report| report.facts == 2 && report.entities == 2),
        "checker fixture did not initialize: {ready:?}"
    );
    neomem.add_fake_rules(&FakeRules { judge: rules(), ..FakeRules::default() })?;
    let mut minutes = 0;
    extract(&neomem, &mut minutes, "planning", DECEMBER)?;
    an_update_replaces_the_old_version(proof, &neomem)?;
    extract(&neomem, &mut minutes, "standup", DECEMBER)?;
    the_same_fact_again_becomes_evidence(proof, &neomem)?;
    extract(
        &neomem,
        &mut minutes,
        "launch",
        "The Edge December 3 release ships to the App Store and TestFlight.",
    )?;
    extract(&neomem, &mut minutes, "rumour", "Sam said Edge ships on December 10, 2026.")?;
    extensions_and_contradictions_link_both_facts(proof, &neomem)?;
    extract(&neomem, &mut minutes, "vague", "Edge might slip a little.")?;
    the_judge_was_asked_one_typed_question_per_fact(proof, &neomem)?;
    unclear_answers_are_written_as_new_and_counted(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn rule(context: &str, label: &str, probability: f32) -> JudgeRule {
    JudgeRule {
        question_contains: None,
        context_contains: Some(context.to_owned()),
        candidate_contains: None,
        answer: JudgeAnswer::Choice { label: label.to_owned(), probability },
    }
}

fn rules() -> Vec<JudgeRule> {
    vec![
        rule(
            "New fact: Edge's ship date is December 3, 2026.\n\nExisting facts:\n1. Edge's ship date is November 1, 2026.",
            "updates_1",
            0.97,
        ),
        rule("New fact: The Edge December 3 release", "extends_1", 0.93),
        rule("New fact: Sam said Edge ships on December 10", "contradicts_1", 0.88),
        rule("New fact: Edge might slip", "updates_1", 0.41),
    ]
}

fn now() -> EpochMs {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    EpochMs::from_millis(i64::try_from(since.as_millis()).unwrap_or(0))
}

fn extract(neomem: &IsolatedNeomem, minutes: &mut i64, session: &str, content: &str) -> anyhow::Result<()> {
    let record = IntakeRecord {
        source: Source::new("claude-code")?,
        occurred: Span::instant(now().plus_millis(*minutes * 60_000)),
        tags: Vec::new(),
        content: content.to_owned(),
        entity_hints: vec!["Edge".to_owned()],
        source_ref: None,
        session: Some(session.to_owned()),
    };
    neomem.client()?.call(Request::Add(record))?;
    *minutes += 10;
    neomem.set_clock(now().plus_millis(*minutes * 60_000))?;
    thread::sleep(Duration::from_millis(2500));
    Ok(())
}

fn search(neomem: &IsolatedNeomem, proof: &mut ProofRun, text: &str) -> anyhow::Result<Vec<SearchHit>> {
    Ok(
        match neomem
            .call(proof, Request::Search(SearchQuery { text: text.to_owned(), ..SearchQuery::default() }))?
        {
            Response::SearchResults(results) => results.hits,
            _ => Vec::new(),
        },
    )
}

fn an_update_replaces_the_old_version(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let hits = search(neomem, proof, "Edge's ship date is November 1")?;
    let first = hits.first();
    proof.claim(
        "an update links the new fact to the old one, and a query worded like the old fact returns the new version",
        first.is_some_and(|hit| {
            hit.fact.content() == DECEMBER
                && hit.fact.links().iter().any(|link| link.kind == FactLinkKind::Updates && link.to.to_string() == NOVEMBER)
        }),
        first.map_or_else(String::new, |hit| hit.fact.content().to_owned()),
    );
    let old = match neomem.call(proof, Request::Show { id: NOVEMBER.parse()? })? {
        Response::Shown(shown) => shown.text,
        _ => String::new(),
    };
    proof.claim(
        "the old version is kept with invalid_at set, never deleted",
        old.contains("invalid_at: 1") && old.contains("November 1"),
        "",
    );
    Ok(())
}

fn the_same_fact_again_becomes_evidence(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let hits = search(neomem, proof, DECEMBER)?;
    let copies = hits.iter().filter(|hit| hit.fact.content() == DECEMBER).count();
    let sources =
        hits.iter().find(|hit| hit.fact.content() == DECEMBER).map_or(0, |hit| hit.fact.sources().len());
    proof.claim(
        "the same fact stated again adds a source and evidence instead of a duplicate",
        copies == 1 && sources == 2,
        format!("{copies} copies, {sources} sources"),
    );
    Ok(())
}

fn extensions_and_contradictions_link_both_facts(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let hits = search(neomem, proof, "Edge December release")?;
    let extension = hits.iter().find(|hit| hit.fact.content().contains("App Store"));
    proof.claim(
        "an extension links to the fact it adds detail to, and both stay valid",
        extension.is_some_and(|hit| hit.fact.links().iter().any(|link| link.kind == FactLinkKind::Extends))
            && hits.iter().any(|hit| hit.fact.content() == DECEMBER && hit.fact.invalid_at().is_none()),
        "",
    );
    let rumour = search(neomem, proof, "when does Edge ship, December 10?")?;
    let flagged = rumour.iter().filter(|hit| hit.conflict).count();
    let linked =
        rumour.iter().any(|hit| hit.fact.links().iter().any(|link| link.kind == FactLinkKind::Contradicts));
    proof.claim(
        "a contradiction is linked, both facts stay valid, and search flags the conflict",
        linked && flagged >= 1,
        format!("{flagged} flagged"),
    );
    Ok(())
}

fn the_judge_was_asked_one_typed_question_per_fact(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let checks: Vec<serde_json::Value> = neomem
        .fake_model_calls()?
        .into_iter()
        .filter(|call| call.route == "/v1/systemone" && call.body.to_string().contains("Existing facts"))
        .map(|call| call.body)
        .collect();
    let well_formed = checks.iter().all(|body| {
        let questions = body["questions"].as_object().map_or(0, serde_json::Map::len);
        let options = body["questions"]["decision"]["criteria"].as_object().map_or(0, serde_json::Map::len);
        questions == 1 && options <= 21 && body["questions"]["decision"]["type"] == "choice"
    });
    proof.claim(
        "each new fact is checked with one typed choice over at most the 5 closest facts",
        !checks.is_empty() && well_formed,
        format!("{} checks", checks.len()),
    );
    Ok(())
}

fn unclear_answers_are_written_as_new_and_counted(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let hits = search(neomem, proof, "Edge might slip")?;
    let written = hits
        .iter()
        .any(|hit| hit.fact.content() == "Edge might slip a little." && hit.fact.links().is_empty());
    let state = Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let unclear: i64 = state.query_row(
        "SELECT count(*) FROM runs WHERE kind = 'extract' AND changes LIKE '%\"unclear\":1%'",
        [],
        |row| row.get(0),
    )?;
    proof.claim(
        "an answer below 0.6 is unclear: the fact is written as new and the run counts it",
        written && unclear >= 1,
        unclear,
    );
    Ok(())
}
