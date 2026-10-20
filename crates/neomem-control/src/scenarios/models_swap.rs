use std::thread;
use std::time::{Duration, Instant};

use neomem_core::ModelRole;
use neomem_proto::{Request, Response};
use rusqlite::{Connection, OpenFlags};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    seed::copy_vault_fixture("edge-ship-date", neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 2, 2)?;
    let output = neomem.neomem(&["models", "use", "planner", "fake/new-planner"])?;
    let planner = neomem.call(
        proof,
        Request::ModelsTest { role: ModelRole::Planner, source: None, structured: false, inputs: Vec::new() },
    )?;
    proof.claim(
        "swapping the planner changes its very next call",
        output.status.success()
            && matches!(&planner, Response::ModelTested(report) if report.model.contains("new-planner")),
        format!("{planner:?}"),
    );
    let output = neomem.neomem(&["models", "use", "embedder", "fake/hash-384"])?;
    proof.claim(
        "models use persists the new embedder assignment",
        output.status.success() && std::fs::read_to_string(neomem.home().config_path())?.contains("hash-384"),
        String::from_utf8_lossy(&output.stderr),
    );
    if output.status.success() {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let report = support::index_report(&neomem, proof)?;
            if report.as_ref().and_then(|report| report.model.as_deref()) == Some("openai:hash-384")
                || Instant::now() > deadline
            {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    let rows = vectors(&neomem)?;
    proof.claim(
        "every fact and entity has a new-model vector and old-model vectors are dropped",
        rows == vec![("openai:hash-384".to_owned(), 4)],
        format!("{rows:?}"),
    );
    let roster = neomem.call(proof, Request::Models)?;
    proof.claim("models shows the new assignment", matches!(&roster, Response::ModelRoster(roles) if roles.iter().any(|role| role.role == ModelRole::Embedder && role.assignment.contains("hash-384"))), format!("{roster:?}"));
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let rows = vectors(&neomem)?;
    proof.claim(
        "the switched model and vectors survive restart",
        rows == vec![("openai:hash-384".to_owned(), 4)],
        format!("{rows:?}"),
    );
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn vectors(neomem: &IsolatedNeomem) -> anyhow::Result<Vec<(String, usize)>> {
    let connection =
        Connection::open_with_flags(neomem.home().index_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement =
        connection.prepare("SELECT model_id, count(*) FROM vectors GROUP BY model_id ORDER BY model_id")?;
    Ok(statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?)
}
