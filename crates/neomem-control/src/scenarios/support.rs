use std::thread;
use std::time::{Duration, Instant};

use neomem_proto::{IndexReport, Request, Response};

use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;

const INDEX_WAIT: Duration = Duration::from_secs(8);

pub fn index_report(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<Option<IndexReport>> {
    Ok(match neomem.call(proof, Request::Status)? {
        Response::Status(status) => status.index,
        _ => None,
    })
}

pub fn wait_for_index(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    facts: usize,
    entities: usize,
) -> anyhow::Result<Option<IndexReport>> {
    let deadline = Instant::now() + INDEX_WAIT;
    loop {
        let report = index_report(neomem, proof)?;
        let ready =
            report.as_ref().is_some_and(|report| report.facts >= facts && report.entities >= entities);
        if ready || Instant::now() > deadline {
            return Ok(report);
        }
        thread::sleep(Duration::from_millis(100));
    }
}
