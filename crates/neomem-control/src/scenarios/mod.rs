mod daemon;
mod intake;
mod models_llm;

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
    Scenario { name: "models-llm", run: models_llm::prove },
];

pub fn find(name: &str) -> Option<Scenario> {
    ALL.iter().copied().find(|scenario| scenario.name == name)
}
