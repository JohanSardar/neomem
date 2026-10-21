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
        if ready {
            return Ok(report);
        }
        if Instant::now() > deadline {
            anyhow::bail!(
                "index did not reach {facts} facts and {entities} entities within {INDEX_WAIT:?}: {report:?}"
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn first_real_id(text: &str) -> Option<String> {
    ["fct_", "ent_", "int_"].iter().find_map(|prefix| {
        text.match_indices(prefix).find_map(|(at, _)| {
            let candidate = text.get(at..at + prefix.len() + 26)?;
            candidate[prefix.len()..]
                .chars()
                .all(|character| character.is_ascii_digit() || character.is_ascii_uppercase())
                .then(|| candidate.to_owned())
        })
    })
}
