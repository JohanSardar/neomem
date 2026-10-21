mod capture;
mod capture_cc;
mod capture_codex;
mod capture_pi;
mod check;
mod contract;
mod correct;
mod daemon;
mod decay;
mod drift;
mod entities;
mod entity;
mod extract;
mod history;
mod index;
mod intake;
mod latest;
mod links;
pub(crate) mod models_embed;
mod models_idle;
mod models_judge;
mod models_llm;
mod models_swap;
mod raw;
mod recall_approve;
mod recall_gate;
mod recall_plan;
pub(crate) mod recall_support;
mod reconcile;
mod search;
mod skill;
mod spool;
pub mod support;
mod vault;

use crate::binaries::Binaries;
use crate::proof::ProofRun;

#[derive(Debug, Clone, Copy)]
pub struct Scenario {
    pub name: &'static str,
    pub run: fn(&mut ProofRun, &Binaries) -> anyhow::Result<()>,
}

pub const ALL: &[Scenario] = &[
    Scenario { name: "contract", run: contract::prove },
    Scenario { name: "recall-gate", run: recall_gate::prove },
    Scenario { name: "recall-plan", run: recall_plan::prove },
    Scenario { name: "recall-approve", run: recall_approve::prove },
    Scenario { name: "capture-cc", run: capture_cc::prove },
    Scenario { name: "capture-codex", run: capture_codex::prove },
    Scenario { name: "capture-pi", run: capture_pi::prove },
    Scenario { name: "models-idle", run: models_idle::prove },
    Scenario { name: "models-swap", run: models_swap::prove },
    Scenario { name: "daemon", run: daemon::prove },
    Scenario { name: "intake", run: intake::prove },
    Scenario { name: "vault", run: vault::prove },
    Scenario { name: "spool", run: spool::prove },
    Scenario { name: "raw", run: raw::prove },
    Scenario { name: "index", run: index::prove },
    Scenario { name: "correct", run: correct::prove },
    Scenario { name: "entity", run: entity::prove },
    Scenario { name: "drift", run: drift::prove },
    Scenario { name: "models-llm", run: models_llm::prove },
    Scenario { name: "models-embed", run: models_embed::prove },
    Scenario { name: "models-judge", run: models_judge::prove },
    Scenario { name: "search", run: search::prove },
    Scenario { name: "extract", run: extract::prove },
    Scenario { name: "check", run: check::prove },
    Scenario { name: "latest", run: latest::prove },
    Scenario { name: "links", run: links::prove },
    Scenario { name: "history", run: history::prove },
    Scenario { name: "decay", run: decay::prove },
    Scenario { name: "reconcile", run: reconcile::prove },
    Scenario { name: "entities", run: entities::prove },
    Scenario { name: "skill", run: skill::prove },
];

pub fn find(name: &str) -> Option<Scenario> {
    ALL.iter().copied().find(|scenario| scenario.name == name)
}
