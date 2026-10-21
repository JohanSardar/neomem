use std::thread;
use std::time::{Duration, Instant};

use neomem_core::ModelRole;
use neomem_proto::{Request, Response, SearchQuery};

use crate::binaries::Binaries;
use crate::isolated::{IsolatedNeomem, Models};
use crate::proof::ProofRun;
use crate::seed;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    "real llama.cpp models".clone_into(&mut proof.models);
    let mut neomem = IsolatedNeomem::prepared_with(binaries, Models::Real)?;
    let config = std::fs::read_to_string(neomem.home().config_path())?;
    let config = config
        .lines()
        .map(|line| {
            if line.starts_with("judge =") {
                "judge = { runtime = \"llama.cpp\", file = \"qwen3-reranker-0.6b-Q8_0.gguf\" }"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(neomem.home().config_path(), config)?;
    neomem.start_daemon()?;
    let before = roster(&neomem, proof)?;
    proof.claim(
        "local models start unloaded",
        before.contains("embedder:unloaded") && before.contains("judge:unloaded"),
        &before,
    );
    seed::copy_vault_fixture("edge-ship-date", neomem.home().root())?;
    let rebuilt = neomem.call(proof, Request::Rebuild)?;
    anyhow::ensure!(
        matches!(&rebuilt, Response::Rebuilt(report) if report.index.facts == 2 && report.index.entities == 2 && report.unreadable_files == 0),
        "idle fixture was not fully indexed: {rebuilt:?}"
    );
    let first = probe(&neomem, proof)?;
    let judge = neomem.call(
        proof,
        Request::ModelsTest { role: ModelRole::Judge, source: None, structured: false, inputs: Vec::new() },
    )?;
    let loaded = roster(&neomem, proof)?;
    proof.claim(
        "the local reranker loads on its first decision",
        matches!(judge, Response::ModelTested(_)) && loaded.contains("judge:loaded"),
        &loaded,
    );
    proof.claim(
        "a search loads the local embedder and finds the current ship date",
        loaded.contains("embedder:loaded")
            && first.as_ref().is_some_and(|facts| facts.iter().any(|fact| fact.contains("November 1"))),
        loaded,
    );
    let now = match neomem.call(proof, Request::Status)? {
        Response::Status(_) => neomem_core::EpochMs::from_millis(i64::try_from(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(),
        )?),
        other => anyhow::bail!("unexpected status {other:?}"),
    };
    neomem.set_clock(now.plus_millis(14 * 60_000))?;
    thread::sleep(Duration::from_millis(500));
    let warm = roster(&neomem, proof)?;
    proof.claim(
        "the default idle timeout exceeds fourteen minutes",
        warm.contains("embedder:loaded") && warm.contains("judge:loaded"),
        warm,
    );
    neomem.set_clock(now.plus_millis(16 * 60_000))?;
    thread::sleep(Duration::from_millis(500));
    let idle = roster(&neomem, proof)?;
    proof.claim(
        "a maintenance tick unloads after the default fifteen minutes",
        idle.contains("embedder:unloaded"),
        idle,
    );
    let added = neomem.neomem(&["add", "idle lifecycle probe", "--source", "note"])?;
    let raw = neomem.neomem(&["raw", "idle lifecycle probe"])?;
    let after_raw = roster(&neomem, proof)?;
    proof.claim(
        "intake and raw search leave models unloaded",
        added.status.success() && raw.status.success() && after_raw.contains("embedder:unloaded"),
        after_raw,
    );
    let started = Instant::now();
    let reloaded = probe(&neomem, proof)?;
    let reload_ms = started.elapsed().as_secs_f64() * 1000.0;
    proof.claim(
        "the next search transparently reloads and preserves its answer; reload latency recorded",
        reloaded == first && reloaded.is_some(),
        format!("reload {reload_ms:.2} ms; {reloaded:?}"),
    );
    let config = std::fs::read_to_string(neomem.home().config_path())?
        .replace("[models]\n", "[models]\nidle_unload = \"30s\"\n");
    std::fs::write(neomem.home().config_path(), config)?;
    neomem.set_clock(now.plus_millis(18 * 60_000))?;
    thread::sleep(Duration::from_millis(500));
    let configured_idle = roster(&neomem, proof)?;
    proof.claim(
        "a configured idle_unload overrides the fifteen-minute default without restarting",
        configured_idle.contains("embedder:unloaded"),
        configured_idle,
    );
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn roster(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<String> {
    Ok(match neomem.call(proof, Request::Models)? {
        Response::ModelRoster(roles) => roles
            .into_iter()
            .map(|role| format!("{}:{}", role.role.name(), role.assignment))
            .collect::<Vec<_>>()
            .join("; "),
        other => format!("{other:?}"),
    })
}

fn probe(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<Option<Vec<String>>> {
    Ok(
        match neomem.call(
            proof,
            Request::Search(SearchQuery { text: "when does Edge ship".to_owned(), ..SearchQuery::default() }),
        )? {
            Response::SearchResults(results) => {
                Some(results.hits.iter().map(|hit| hit.fact.content().to_owned()).collect())
            }
            _ => None,
        },
    )
}
