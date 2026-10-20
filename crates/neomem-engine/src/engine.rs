use std::path::PathBuf;
use std::sync::Arc;

use neomem_core::{Clock, EpochMs};
use neomem_proto::{Request, Response};
use neomem_state::StateDb;

use crate::ids::IdMint;

#[derive(Debug)]
pub struct EngineParts {
    pub home: PathBuf,
    pub state_db: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub daemon_version: String,
}

#[derive(Debug)]
pub struct Engine {
    pub(crate) home: PathBuf,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) daemon_version: String,
    pub(crate) started_at: EpochMs,
    pub(crate) state: StateDb,
    pub(crate) ids: IdMint,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("the engine could not open state.db: {0}")]
    State(#[from] neomem_state::StateError),
}

impl Engine {
    pub fn open(parts: EngineParts) -> Result<Self, EngineError> {
        let started_at = parts.clock.now();
        Ok(Self {
            home: parts.home,
            state: StateDb::open(&parts.state_db)?,
            clock: parts.clock,
            daemon_version: parts.daemon_version,
            started_at,
            ids: IdMint::new(),
        })
    }

    pub fn handle(&self, request: Request) -> Response {
        match request {
            Request::Status => Response::Status(self.status()),
            Request::Shutdown => {
                self.flush();
                Response::ShuttingDown
            }
            Request::Add(record) => self.add(record),
        }
    }

    pub fn flush(&self) {
        tracing::info!("engine flushed");
    }
}
