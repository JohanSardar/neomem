use neomem_proto::{PROTOCOL_VERSION, StatusReport};

use crate::Engine;

impl Engine {
    pub(crate) fn status(&self) -> StatusReport {
        StatusReport {
            extraction: self.state.extraction_counts().ok(),
            daemon_version: self.daemon_version.clone(),
            protocol: PROTOCOL_VERSION,
            home: self.home.display().to_string(),
            pid: std::process::id(),
            uptime_ms: self.clock.now().millis_since(self.started_at),
            index: Some(self.index_report()),
        }
    }
}
