use crate::binaries::Binaries;
use crate::proof::ProofRun;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    super::capture::prove(proof, binaries, "codex")
}
