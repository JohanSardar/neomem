use std::thread;
use std::time::{Duration, Instant};

use neomem_core::ModelRole;
use neomem_proto::{Request, Response};

use crate::binaries::Binaries;
use crate::isolated::{IsolatedNeomem, Models};
use crate::proof::ProofRun;

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
    proof.claim("local models start unloaded", before.contains("unloaded"), &before);
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
        "a query loads the local embedder",
        loaded.contains("embedder:loaded") && first.is_some(),
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
        "the next query transparently reloads and preserves its answer; reload latency recorded",
        reloaded == first && reloaded.is_some(),
        format!("reload {reload_ms:.2} ms; {reloaded:?}"),
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

fn probe(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<Option<Vec<f32>>> {
    Ok(
        match neomem.call(
            proof,
            Request::ModelsTest {
                role: ModelRole::Embedder,
                source: None,
                structured: false,
                inputs: vec!["when does Edge ship".to_owned(), "Edge ships on November 1".to_owned()],
            },
        )? {
            Response::ModelTested(report) => Some(report.scores),
            _ => None,
        },
    )
}
