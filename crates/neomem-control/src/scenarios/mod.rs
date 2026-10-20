mod correct;
mod daemon;
mod drift;
mod entity;
mod index;
mod intake;
pub(crate) mod models_embed;
mod models_judge;
mod models_llm;
mod raw;
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
];

pub fn find(name: &str) -> Option<Scenario> {
    ALL.iter().copied().find(|scenario| scenario.name == name)
}
