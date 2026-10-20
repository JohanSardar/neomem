use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use neomem_proto::{IndexReport, Request, Response};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::seed;

const OCTOBER: &str = "fct_01M56PJAD08JBX4FR84G98PBSK";
const NOVEMBER: &str = "fct_01M59NY2N09AC5V7E9Z0C1HT0H";
const WAIT: Duration = Duration::from_secs(8);

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    seed::copy_vault_fixture("edge-ship-date", neomem.home().root())?;
    seeded_files_reach_the_index(proof, &neomem)?;
    the_latest_version_is_recorded(proof, &neomem)?;
    a_new_file_is_indexed_without_a_rebuild(proof, &neomem)?;
    rebuild_reproduces_the_same_rows(proof, &neomem)?;
    a_restart_loads_vectors_instead_of_re_embedding(proof, &mut neomem)?;
    a_lost_index_is_rebuilt_from_the_files(proof, &mut neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn index_report(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<Option<IndexReport>> {
    Ok(match neomem.call(proof, Request::Status)? {
        Response::Status(status) => status.index,
        _ => None,
    })
}

fn wait_for_facts(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    facts: usize,
) -> anyhow::Result<Option<IndexReport>> {
    let deadline = Instant::now() + WAIT;
    loop {
        let report = index_report(neomem, proof)?;
        if report.as_ref().is_some_and(|report| report.facts >= facts) || Instant::now() > deadline {
            return Ok(report);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn index_db(neomem: &IsolatedNeomem) -> anyhow::Result<Connection> {
    Ok(Connection::open_with_flags(neomem.home().index_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

fn rows(neomem: &IsolatedNeomem) -> anyhow::Result<Vec<String>> {
    let connection = index_db(neomem)?;
    let mut dump = Vec::new();
    for query in [
        "SELECT id || '|' || kind || '|' || content || '|' || ifnull(latest_id, '') FROM memories ORDER BY id",
        "SELECT memory_id || '|' || model_id || '|' || length(data) FROM vectors ORDER BY memory_id",
        "SELECT from_id || '|' || to_id || '|' || type FROM links ORDER BY from_id, to_id, type",
        "SELECT name || '|' || entity_id FROM names ORDER BY name, entity_id",
    ] {
        let mut statement = connection.prepare(query)?;
        let found: Vec<String> = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
        dump.extend(found);
    }
    Ok(dump)
}

fn seeded_files_reach_the_index(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let report = wait_for_facts(neomem, proof, 2)?;
    let holds = report.as_ref().is_some_and(|report| {
        report.facts == 2
            && report.entities == 2
            && report.vectors == 4
            && report.names == 3
            && report.links >= 4
    });
    proof.claim(
        "a seeded fact chain reaches the index through the file watcher: 2 facts, 2 entities, 4 vectors, 3 names",
        holds,
        format!("{report:?}"),
    );
    Ok(())
}

fn the_latest_version_is_recorded(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let latest: Option<String> = index_db(neomem)?
        .query_row("SELECT latest_id FROM memories WHERE id = ?1", [OCTOBER], |row| row.get(0))
        .optional()?;
    proof.claim(
        "index.db records the newest fact of the updates chain as latest_id of the old one",
        latest.as_deref() == Some(NOVEMBER),
        format!("{latest:?}"),
    );
    Ok(())
}

fn a_new_file_is_indexed_without_a_rebuild(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let november = neomem
        .home()
        .root()
        .join("facts/2026/10")
        .join(format!("{NOVEMBER}_edge-s-ship-date-is-november-1-2026.md"));
    let extension = fs::read_to_string(&november)?
        .replace(NOVEMBER, "fct_01M59NY2N09AC5V7E9Z0C1HT0J")
        .replace("Edge's ship date is November 1, 2026.", "The Edge launch event is on November 2, 2026.")
        .replace(
            "links: [{type: updates, to: fct_01M56PJAD08JBX4FR84G98PBSK}]",
            "links: [{type: extends, to: fct_01M59NY2N09AC5V7E9Z0C1HT0H}]",
        );
    fs::write(
        neomem.home().root().join("facts/2026/10/fct_01M59NY2N09AC5V7E9Z0C1HT0J_edge-launch-event.md"),
        extension,
    )?;
    let report = wait_for_facts(neomem, proof, 3)?;
    proof.claim(
        "a hand-written fact file is indexed within seconds, with its vector, and no rebuild",
        report.as_ref().is_some_and(|report| report.facts == 3 && report.vectors == 5),
        format!("{report:?}"),
    );
    Ok(())
}

fn rebuild_reproduces_the_same_rows(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let before = rows(neomem)?;
    let output = neomem.neomem(&["rebuild", "--json"])?;
    let reply: Option<Response> = serde_json::from_slice(&output.stdout).ok();
    let after = rows(neomem)?;
    proof.claim(
        "neomem rebuild re-creates index.db from the files with identical rows",
        matches!(reply, Some(Response::Rebuilt(_))) && before == after && !before.is_empty(),
        format!(
            "{} rows before, {} after; {}",
            before.len(),
            after.len(),
            String::from_utf8_lossy(&output.stdout).trim()
        ),
    );
    Ok(())
}

fn embedding_calls(neomem: &IsolatedNeomem) -> anyhow::Result<usize> {
    Ok(neomem.fake_model_calls()?.iter().filter(|call| call.route == "/v1/embeddings").count())
}

fn a_restart_loads_vectors_instead_of_re_embedding(
    proof: &mut ProofRun,
    neomem: &mut IsolatedNeomem,
) -> anyhow::Result<()> {
    let before = embedding_calls(neomem)?;
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let report = wait_for_facts(neomem, proof, 3)?;
    let after = embedding_calls(neomem)?;
    proof.claim(
        "a restart loads the index and its vectors from index.db without embedding the memories again",
        report.as_ref().is_some_and(|report| report.facts == 3 && report.vectors == 5) && after - before <= 1,
        format!("embedding calls {before} -> {after} (one dimension probe allowed), {report:?}"),
    );
    Ok(())
}

fn a_lost_index_is_rebuilt_from_the_files(
    proof: &mut ProofRun,
    neomem: &mut IsolatedNeomem,
) -> anyhow::Result<()> {
    neomem.stop_daemon()?;
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", neomem.home().index_db_path().display()));
    }
    neomem.start_daemon()?;
    let report = wait_for_facts(neomem, proof, 3)?;
    proof.claim(
        "deleting index.db loses nothing: the next start rebuilds it from the files",
        report
            .as_ref()
            .is_some_and(|report| report.facts == 3 && report.entities == 2 && report.vectors == 5),
        format!("{report:?}"),
    );
    Ok(())
}
