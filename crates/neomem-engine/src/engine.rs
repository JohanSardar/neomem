use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use neomem_core::{Clock, Embedder, EpochMs, ModelRoles};
use neomem_index::Index;
use neomem_proto::{Request, Response};
use neomem_state::StateDb;
use neomem_vault::{CommitPolicy, Vault};

use crate::ids::IdMint;
use crate::indexing::IndexHealth;
use crate::models::ModelProbe;

#[derive(Debug)]
pub struct EngineParts {
    pub home: PathBuf,
    pub state_db: PathBuf,
    pub index_db: PathBuf,
    pub commit_policy: CommitPolicy,
    pub clock: Arc<dyn Clock>,
    pub models: Arc<dyn ModelRoles>,
    pub daemon_version: String,
}

#[derive(Debug)]
pub struct Engine {
    pub(crate) home: PathBuf,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) daemon_version: String,
    pub(crate) started_at: EpochMs,
    pub(crate) state: StateDb,
    pub(crate) vault: Vault,
    pub(crate) commit_policy: CommitPolicy,
    pub(crate) ids: IdMint,
    pub(crate) models: Arc<dyn ModelRoles>,
    pub(crate) embedder: Option<Arc<dyn Embedder>>,
    pub(crate) index: Index,
    pub(crate) health: Mutex<IndexHealth>,
    pub(crate) writer: Mutex<()>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("the engine could not open state.db: {0}")]
    State(#[from] neomem_state::StateError),
    #[error("the engine could not open the vault: {0}")]
    Vault(#[from] neomem_vault::VaultError),
    #[error("the engine could not open index.db: {0}")]
    Index(#[from] neomem_index::IndexError),
}

impl Engine {
    pub fn open(parts: EngineParts) -> Result<Self, EngineError> {
        let started_at = parts.clock.now();
        let embedder = match parts.models.embedder() {
            Ok(embedder) => Some(embedder),
            Err(error) => {
                tracing::warn!("search runs without vectors until the embedder works: {error}");
                None
            }
        };
        let model = embedder.as_ref().map(|embedder| (embedder.model_id(), embedder.dims()));
        let opened = Index::open(&parts.index_db, model)?;
        let health = IndexHealth {
            needs_rebuild: opened.needs_rebuild,
            missing_vectors: opened.missing_vectors.len(),
        };
        Ok(Self {
            embedder,
            index: opened.index,
            health: Mutex::new(health),
            writer: Mutex::new(()),
            vault: Vault::open(&parts.home)?,
            home: parts.home,
            state: StateDb::open(&parts.state_db)?,
            commit_policy: parts.commit_policy,
            clock: parts.clock,
            daemon_version: parts.daemon_version,
            started_at,
            ids: IdMint::new(),
            models: parts.models,
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
            Request::Raw(query) => self.raw(&query),
            Request::Correct { intake, reason } => self.correct(intake, &reason),
            Request::Rebuild => self.rebuild(),
            Request::Models => self.model_roster(),
            Request::ModelsTest { role, source, structured } => {
                self.test_model(ModelProbe { role, source: source.as_ref(), structured })
            }
            Request::Login { provider } => self.start_login(provider),
            Request::LoginFinish { provider } => self.finish_login(provider),
        }
    }

    pub fn flush(&self) {
        self.commit_now();
        tracing::info!("engine flushed");
    }
}
