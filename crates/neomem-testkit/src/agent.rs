use std::path::Path;

use neomem_core::{AgentAdapter, CaptureAgent, CaptureError, Captured};

#[derive(Debug, Clone)]
pub struct RecordedAgent {
    pub agent: CaptureAgent,
    pub transcript: Captured,
}

impl AgentAdapter for RecordedAgent {
    fn agent(&self) -> CaptureAgent {
        self.agent
    }

    fn read_new(&self, _file: &Path, position: u64) -> Result<Captured, CaptureError> {
        let mut captured = self.transcript.clone();
        if position >= captured.position {
            captured.records.clear();
        }
        Ok(captured)
    }
}

pub fn agent_contract(adapter: &dyn AgentAdapter, file: &Path) -> Result<(), CaptureError> {
    let first = adapter.read_new(file, 0)?;
    let next = adapter.read_new(file, first.position)?;
    if first.records.is_empty()
        || !next.records.is_empty()
        || next.position != first.position
        || adapter.slice(file)? != first.records
    {
        return Err(CaptureError("agent adapter violates incremental read/slice contract".to_owned()));
    }
    Ok(())
}
