use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{EpochMs, FactId, ModelRole};
use neomem_proto::{Request, Response, SearchQuery};
use neomem_testkit::FakeRules;
use rusqlite::{Connection, OpenFlags};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;

const OLD: &str = "openai:hash-256";
const NEW: &str = "openai:hash-384";
const MEMORIES: i64 = 100;
const NOVEMBER: &str = "fct_01M59NY2N09AC5V7E9Z0C1HT0H";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    seed_memories(&neomem)?;
    support::wait_for_index(&neomem, proof, 98, 2)?;
    let output = neomem.neomem(&["models", "use", "planner", "new-planner"])?;
    let planner = neomem.call(
        proof,
        Request::ModelsTest { role: ModelRole::Planner, source: None, structured: false, inputs: Vec::new() },
    )?;
    proof.claim(
        "swapping the planner changes its very next call",
        output.status.success()
            && matches!(&planner, Response::ModelTested(report) if report.model.contains("new-planner"))
            && neomem.fake_model_calls()?.iter().any(|call| call.body["model"] == "new-planner"),
        format!("{planner:?}"),
    );
    let delayed_embeddings =
        FakeRules { latency_ms: [("/v1/embeddings".to_owned(), 100)].into(), ..FakeRules::default() };
    neomem.add_fake_rules(&delayed_embeddings)?;
    let output = neomem.neomem(&["models", "use", "embedder", "fake/hash-384"])?;
    proof.claim(
        "models use persists the new embedder assignment",
        output.status.success() && fs::read_to_string(neomem.home().config_path())?.contains("hash-384"),
        String::from_utf8_lossy(&output.stderr),
    );
    let partial = wait_for_partial(&neomem)?;
    proof.claim(
        "background re-embedding keeps complete old vectors alongside a partial new set",
        partial.as_ref().is_some_and(|state| {
            state.active == OLD && state.old == MEMORIES && state.new > 0 && state.new < MEMORIES
        }),
        format!("{partial:?}"),
    );
    let during = search(&neomem, proof)?;
    proof.claim("search keeps answering while re-embedding", during.0 && during.2 == OLD, during.1);
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let resumed = state(&neomem)?;
    proof.claim(
        "a restart mid-swap retains the active old model and staged vectors",
        resumed.active == OLD && resumed.old == MEMORIES && resumed.new > 0,
        format!("{resumed:?}"),
    );
    let (complete, atomic, answered) = wait_for_switch(&neomem, proof)?;
    proof.claim(
        "search answers throughout the remaining rebuild and the complete set activates in one step",
        atomic && answered && complete.active == NEW,
        format!("{complete:?}; atomic {atomic}, answered {answered}"),
    );
    proof.claim(
        "every fact and entity has only a vector for the new model after the switch",
        complete.new == MEMORIES && complete.old == 0 && complete.new_dims == 384,
        format!("{complete:?}"),
    );
    let after = search(&neomem, proof)?;
    proof.claim(
        "a query worded like a stored fact still finds it after the switch",
        after.0 && after.2 == NEW,
        after.1,
    );
    let roster = neomem.call(proof, Request::Models)?;
    proof.claim("models shows the new assignment", matches!(&roster, Response::ModelRoster(roles) if roles.iter().any(|role| role.role == ModelRole::Embedder && role.assignment.contains("hash-384"))), format!("{roster:?}"));
    let config = fs::read_to_string(neomem.home().config_path())?.replace("new-planner", "edited-planner");
    fs::write(neomem.home().config_path(), config)?;
    let planner = neomem.call(
        proof,
        Request::ModelsTest { role: ModelRole::Planner, source: None, structured: false, inputs: Vec::new() },
    )?;
    proof.claim(
        "a direct config edit also reloads a role on its next call",
        matches!(&planner, Response::ModelTested(report) if report.model.contains("edited-planner")),
        format!("{planner:?}"),
    );
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn seed_memories(neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    seed::copy_vault_fixture("edge-ship-date", neomem.home().root())?;
    let directory = neomem.home().root().join("facts/2026/10");
    let template =
        fs::read_to_string(directory.join(format!("{NOVEMBER}_edge-s-ship-date-is-november-1-2026.md")))?;
    for number in 0..96 {
        let id = FactId::from_parts(EpochMs::from_millis(1_792_540_800_000), 1000 + number);
        fs::write(
            directory.join(format!("{id}_edge-swap-fixture.md")),
            template.replace(NOVEMBER, &id.to_string()),
        )?;
    }
    Ok(())
}

#[derive(Debug)]
struct VectorState {
    active: String,
    old: i64,
    new: i64,
    new_dims: i64,
}

fn state(neomem: &IsolatedNeomem) -> anyhow::Result<VectorState> {
    let mut connection =
        Connection::open_with_flags(neomem.home().index_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let transaction = connection.transaction()?;
    Ok(VectorState {
        active: transaction
            .query_row("SELECT model FROM active_model WHERE singleton = 1", [], |row| row.get(0))?,
        old: transaction
            .query_row("SELECT count(*) FROM vectors WHERE model_id = ?1", [OLD], |row| row.get(0))?,
        new: transaction
            .query_row("SELECT count(*) FROM vectors WHERE model_id = ?1", [NEW], |row| row.get(0))?,
        new_dims: transaction.query_row(
            "SELECT coalesce(min(length(data) / 2), 0) FROM vectors WHERE model_id = ?1",
            [NEW],
            |row| row.get(0),
        )?,
    })
}

fn wait_for_partial(neomem: &IsolatedNeomem) -> anyhow::Result<Option<VectorState>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = state(neomem)?;
        if state.new > 0 && state.new < MEMORIES {
            return Ok(Some(state));
        }
        if Instant::now() > deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn search(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<(bool, String, String)> {
    Ok(
        match neomem.call(
            proof,
            Request::Search(SearchQuery {
                text: "Edge's ship date is November 1, 2026.".to_owned(),
                k: Some(1),
                ..SearchQuery::default()
            }),
        )? {
            Response::SearchResults(results) => {
                let found = results.hits.iter().any(|hit| hit.fact.content().contains("November 1"));
                (
                    found,
                    format!("{}; {} hits; found ship date {found}", results.model, results.hits.len()),
                    results.model,
                )
            }
            other => (false, format!("{other:?}"), String::new()),
        },
    )
}

fn wait_for_switch(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
) -> anyhow::Result<(VectorState, bool, bool)> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut atomic = true;
    let mut answered = true;
    loop {
        let state = state(neomem)?;
        atomic &= match state.active.as_str() {
            OLD => state.old == MEMORIES,
            NEW => state.new == MEMORIES,
            _ => false,
        };
        let response = search(neomem, proof)?;
        answered &= response.0 && matches!(response.2.as_str(), OLD | NEW);
        if (state.active == NEW && state.old == 0) || Instant::now() > deadline {
            return Ok((state, atomic, answered));
        }
        thread::sleep(Duration::from_millis(10));
    }
}
