mod daemon;
mod intake;
mod raw;
mod spool;
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
];

pub fn find(name: &str) -> Option<Scenario> {
    ALL.iter().copied().find(|scenario| scenario.name == name)
}
