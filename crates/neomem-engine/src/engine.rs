use std::path::PathBuf;
use std::sync::Arc;

use neomem_core::{Clock, EpochMs};
use neomem_proto::{Request, Response};

#[derive(Debug)]
pub struct EngineParts {
    pub home: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub daemon_version: String,
}

#[derive(Debug)]
pub struct Engine {
    pub(crate) home: PathBuf,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) daemon_version: String,
    pub(crate) started_at: EpochMs,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("the engine could not open {what}: {reason}")]
    Open { what: &'static str, reason: String },
}

impl Engine {
    pub fn open(parts: EngineParts) -> Result<Self, EngineError> {
        let started_at = parts.clock.now();
        Ok(Self { home: parts.home, clock: parts.clock, daemon_version: parts.daemon_version, started_at })
    }

    #[expect(clippy::needless_pass_by_value)]
    pub fn handle(&self, request: Request) -> Response {
        match request {
            Request::Status => Response::Status(self.status()),
            Request::Shutdown => {
                self.flush();
                Response::ShuttingDown
            }
        }
    }

    pub fn flush(&self) {
        tracing::info!("engine flushed");
    }
}
