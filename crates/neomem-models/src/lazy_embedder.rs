use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Duration;

use neomem_core::{Embedder, EpochMs, ModelError, ModelId, Purpose, Vector};

use crate::llama::local_model_id;
use crate::local_slot::LocalSlot;
use crate::query_cache::{QUERY_CACHE_ENTRIES, QueryCache};
use crate::{LlamaEmbedder, gguf};

#[derive(Debug)]
pub(crate) struct LazyEmbedder {
    path: PathBuf,
    dims: usize,
    loaded: LocalSlot<QueryCache>,
}

impl LazyEmbedder {
    pub(crate) fn at(path: PathBuf, last_read: Arc<AtomicI64>) -> Result<Self, ModelError> {
        Ok(Self { dims: gguf::embedding_dims(&path)?, path, loaded: LocalSlot::new(last_read) })
    }

    pub(crate) fn maintain(&self, now: EpochMs, timeout: Duration) {
        self.loaded.maintain(now, timeout);
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded.is_loaded()
    }
}

impl Embedder for LazyEmbedder {
    fn embed(&self, texts: &[&str], purpose: Purpose) -> Result<Vec<Vector>, ModelError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.loaded.run(
            || Ok(QueryCache::around(Arc::new(LlamaEmbedder::open(&self.path)?), QUERY_CACHE_ENTRIES)),
            |embedder| embedder.embed(texts, purpose),
        )
    }

    fn model_id(&self) -> ModelId {
        local_model_id(&self.path)
    }

    fn dims(&self) -> usize {
        self.dims
    }
}
