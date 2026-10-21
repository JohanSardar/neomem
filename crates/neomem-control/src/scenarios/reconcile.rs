use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{EntityId, EpochMs, FactId, FactLinkKind, IntakeId, IntakeRecord, Source, Span};
use neomem_proto::{DreamReport, Request, Response, SearchQuery};
use neomem_testkit::{FakeRules, LlmRule, RuleToolCall};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;

const RECONCILER: &str = "You are neomem's fact reconciler";
const MINUTE: i64 = 60_000;

struct Seeded {
    keep: FactId,
    duplicate: FactId,
    newer: FactId,
    older: FactId,
    source: IntakeId,
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let seeded = seed(&neomem)?;
    support::wait_for_index(&neomem, proof, 4, 1)?;
    neomem.add_fake_rules(&FakeRules { llm: scripted_reconciler(), ..FakeRules::default() })?;
    let report = dream(&neomem, proof)?;
    duplicates_merge_and_updates_apply(proof, &neomem, &seeded)?;
    proof.claim(
        "an invented fact number and a tool outside the reconciler's set are both rejected",
        report
            .as_ref()
            .is_some_and(|report| report.reconciler.rejected == 2 && report.reconciler.writes == 2),
        format!("{report:?}"),
    );
    the_reconciler_sees_numbers_and_only_its_tools(proof, &neomem)?;
    the_round_cap_holds(proof, &neomem)?;
    triggers_start_runs_on_their_own(proof, &neomem, seeded.source)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(0)
}

fn seed(neomem: &IsolatedNeomem) -> anyhow::Result<Seeded> {
    let record = IntakeRecord {
        source: Source::new("note")?,
        occurred: Span::instant(EpochMs::from_millis(now_ms() - 10 * MINUTE)),
        tags: Vec::new(),
        content: "Johan: the Edge beta has 50 testers".to_owned(),
        entity_hints: Vec::new(),
        source_ref: None,
        session: None,
    };
    let Response::Added(receipt) = neomem.client()?.call(Request::Add(record))? else {
        anyhow::bail!("add failed")
    };
    let home = neomem.home().root();
    let edge = EntityId::from_parts(EpochMs::from_millis(1_780_000_000_000), 7);
    fs::create_dir_all(home.join("entities/project"))?;
    fs::write(
        home.join("entities/project/edge.md"),
        format!(
            "---\nid: {edge}\nkind: entity\nentity_kind: project\nname: Edge\naliases: []\nrecorded_at: 1780000000000\nupdated_at: 1780000000000\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
        ),
    )?;
    let now = now_ms();
    let fact_id =
        |minutes: i64, seed: u128| FactId::from_parts(EpochMs::from_millis(now - minutes * MINUTE), seed);
    let seeded = Seeded {
        keep: fact_id(1, 1),
        duplicate: fact_id(2, 2),
        newer: fact_id(3, 3),
        older: fact_id(4, 4),
        source: receipt.id,
    };
    let other_source = IntakeId::from_parts(EpochMs::from_millis(now - 20 * MINUTE), 9);
    for (id, minutes, content, source) in [
        (seeded.keep, 1, "Edge's beta has 50 testers.", seeded.source),
        (seeded.duplicate, 2, "Edge's beta has fifty testers.", other_source),
        (seeded.newer, 3, "Edge ships on December 3, 2026.", other_source),
        (seeded.older, 4, "Edge ships on November 1, 2026.", other_source),
    ] {
        write_fact(home, id, edge, now - minutes * MINUTE, content, source)?;
    }
    Ok(seeded)
}

fn write_fact(
    home: &Path,
    id: FactId,
    edge: EntityId,
    recorded: i64,
    content: &str,
    source: IntakeId,
) -> anyhow::Result<()> {
    fs::create_dir_all(home.join("facts/2026/10"))?;
    fs::write(
        home.join(format!("facts/2026/10/{id}_reconcile.md")),
        format!(
            "---\nid: {id}\nkind: fact\ntags: [code]\nabout: [{edge}]\nrecorded_at: {recorded}\nupdated_at: {recorded}\nvalid_at: {recorded}\ninvalid_at: null\nimportance: 5\ndurability: ordinary\nsources: [{source}]\nlinks: []\nevidence: []\n---\n{content}\n"
        ),
    )?;
    Ok(())
}

fn call(name: &str, arguments: serde_json::Value) -> RuleToolCall {
    RuleToolCall { name: name.to_owned(), arguments }
}

fn scripted_reconciler() -> Vec<LlmRule> {
    vec![
        LlmRule {
            system_contains: Some(RECONCILER.to_owned()),
            user_contains: Some("fifty testers".to_owned()),
            step: Some(0),
            times: Some(1),
            tool_calls: vec![
                call("merge_facts", json!({ "keep": 1, "duplicate": 2 })),
                call("link", json!({ "from": 3, "kind": "updates", "to": 4 })),
                call("link", json!({ "from": 99, "kind": "extends", "to": 1 })),
                call(
                    "create_entity",
                    json!({ "name": "Beta", "kind": "thing", "aliases": [], "facts": [1] }),
                ),
            ],
            ..LlmRule::default()
        },
        LlmRule {
            system_contains: Some(RECONCILER.to_owned()),
            user_contains: Some("fifty testers".to_owned()),
            step: Some(4),
            times: Some(1),
            reply: "Merged the duplicate and applied the update.".to_owned(),
            ..LlmRule::default()
        },
    ]
}

fn dream(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<Option<DreamReport>> {
    Ok(match neomem.call(proof, Request::Dream)? {
        Response::Dreamed(report) => Some(report),
        _ => None,
    })
}

fn fact_text(neomem: &IsolatedNeomem, proof: &mut ProofRun, id: FactId) -> anyhow::Result<String> {
    Ok(match neomem.call(proof, Request::Show { id: id.into() })? {
        Response::Shown(shown) => shown.text,
        _ => String::new(),
    })
}

fn duplicates_merge_and_updates_apply(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    seeded: &Seeded,
) -> anyhow::Result<()> {
    let keep = fact_text(neomem, proof, seeded.keep)?;
    let duplicate = fact_text(neomem, proof, seeded.duplicate)?;
    proof.claim(
        "merge_facts keeps one fact with both sources, links it to the duplicate, and keeps the duplicate's file marked replaced",
        keep.contains(&format!("{{type: updates, to: {}}}", seeded.duplicate)) && keep.matches("int_").count() == 2 && duplicate.contains("invalid_at: 1"),
        "",
    );
    let hits = match neomem.call(
        proof,
        Request::Search(SearchQuery { text: "when does Edge ship".to_owned(), ..SearchQuery::default() }),
    )? {
        Response::SearchResults(results) => results.hits,
        _ => Vec::new(),
    };
    let ship: Vec<FactId> =
        hits.iter().filter(|hit| hit.fact.content().contains("ships")).map(|hit| hit.fact.id()).collect();
    let linked = hits.iter().any(|hit| {
        hit.fact.id() == seeded.newer
            && hit
                .fact
                .links()
                .iter()
                .any(|link| link.kind == FactLinkKind::Updates && link.to == seeded.older)
    });
    proof.claim(
        "an update link from the reconciler leaves only the newer ship date in search",
        ship == vec![seeded.newer] && linked,
        format!("{ship:?}"),
    );
    Ok(())
}

fn the_reconciler_sees_numbers_and_only_its_tools(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let requests: Vec<serde_json::Value> = neomem
        .fake_model_calls()?
        .into_iter()
        .filter(|call| call.route == "/v1/chat/completions" && call.body.to_string().contains(RECONCILER))
        .map(|call| call.body)
        .collect();
    let tools: Vec<String> = requests
        .first()
        .and_then(|body| body["tools"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect();
    let expected = [
        "search_facts",
        "recent_facts",
        "search_intake",
        "follow_links",
        "link",
        "invalidate",
        "merge_facts",
    ];
    proof.claim(
        "the reconciler is offered exactly its tools: reads plus link, invalidate and merge, and no delete",
        tools.len() == expected.len() && expected.iter().all(|name| tools.iter().any(|tool| tool == name)),
        format!("{tools:?}"),
    );
    let leak = requests.iter().find_map(|body| support::first_real_id(&body.to_string()));
    proof.claim(
        "it only ever sees short numbers, never a real ID",
        !requests.is_empty() && leak.is_none(),
        leak.unwrap_or_else(|| format!("{} requests", requests.len())),
    );
    Ok(())
}

fn the_round_cap_holds(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let home = neomem.home().root();
    let edge = EntityId::from_parts(EpochMs::from_millis(1_780_000_000_000), 7);
    write_fact(
        home,
        FactId::from_parts(EpochMs::from_millis(now_ms()), 20),
        edge,
        now_ms(),
        "Edge has a waitlist of 300 people.",
        IntakeId::from_parts(EpochMs::from_millis(1), 1),
    )?;
    support::wait_for_index(neomem, proof, 5, 1)?;
    neomem.add_fake_rules(&FakeRules {
        llm: vec![LlmRule {
            system_contains: Some(RECONCILER.to_owned()),
            tool_calls: vec![call("recent_facts", json!({ "limit": 3 }))],
            ..LlmRule::default()
        }],
        ..FakeRules::default()
    })?;
    let report = dream(neomem, proof)?;
    proof.claim(
        "a reconciler that never stops calling tools is cut off at 12 rounds",
        report.as_ref().is_some_and(|report| report.reconciler.rounds == 12),
        format!("{:?}", report.map(|report| report.reconciler)),
    );
    Ok(())
}

fn consolidation_triggers(neomem: &IsolatedNeomem) -> anyhow::Result<Vec<String>> {
    let state = Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = state.prepare(
        "SELECT json_extract(changes, '$.trigger') FROM runs WHERE kind = 'consolidation' ORDER BY id",
    )?;
    Ok(statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?)
}

fn wait_for_trigger(neomem: &IsolatedNeomem, trigger: &str) -> anyhow::Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if consolidation_triggers(neomem)?.iter().any(|seen| seen == trigger) {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(250));
    }
    Ok(false)
}

fn triggers_start_runs_on_their_own(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    source: IntakeId,
) -> anyhow::Result<()> {
    neomem.client()?.call(Request::Correct {
        intake: source,
        reason: "the beta numbers were from another app".to_owned(),
    })?;
    proof.claim(
        "a correction touching a reconciled fact starts a consolidation run on its own",
        wait_for_trigger(neomem, "late data or a correction")?,
        format!("{:?}", consolidation_triggers(neomem)?),
    );
    let records_with_deferred_batch = 50 + 20 - 1;
    for number in 0..records_with_deferred_batch {
        let record = IntakeRecord {
            source: Source::new("standup-digest")?,
            occurred: Span::instant(EpochMs::from_millis(now_ms())),
            tags: Vec::new(),
            content: format!("Standup note {number}: Edge task {number} is done"),
            entity_hints: vec!["Edge".to_owned()],
            source_ref: None,
            session: None,
        };
        neomem.client()?.call(Request::Add(record))?;
    }
    proof.claim(
        "50 new facts start a consolidation run on their own",
        wait_for_trigger(neomem, "50 new facts")?,
        format!("{:?}", consolidation_triggers(neomem)?),
    );
    Ok(())
}
