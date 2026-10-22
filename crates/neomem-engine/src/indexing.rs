use std::path::PathBuf;
use std::sync::{Arc, PoisonError};
use std::time::Instant;

use neomem_core::{Embedder, Memory, ModelError, Purpose};
use neomem_index::{Change, IndexError};
use neomem_proto::{FailureKind, IndexReport, RebuildReport, Response};

use crate::Engine;
use crate::reply::failed;

const EMBED_BATCH: usize = 32;

#[derive(Debug, Default)]
pub(crate) struct IndexHealth {
    pub(crate) needs_rebuild: bool,
    pub(crate) missing_vectors: usize,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IndexingError {
    #[error("embedding failed: {0}")]
    Embedding(#[from] ModelError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    State(#[from] neomem_state::StateError),
    #[error("writing the memory file failed: {0}")]
    Vault(#[from] neomem_vault::VaultError),
}

impl Engine {
    pub(crate) fn index_memories(&self, located: Vec<(Memory, PathBuf)>) -> Result<(), IndexingError> {
        let _writing = self.lock_writer();
        let changes = self.embedded(located)?;
        self.index.apply(&changes)?;
        Ok(())
    }

    pub(crate) fn rebuild(&self) -> Response {
        match self.rebuild_index() {
            Ok(report) => Response::Rebuilt(report),
            Err(error) => failed(FailureKind::Internal, format!("rebuilding the index failed: {error}")),
        }
    }

    pub(crate) fn rebuild_index(&self) -> Result<RebuildReport, IndexingError> {
        let _writing = self.lock_writer();
        let started = Instant::now();
        let mut unreadable_files = 0;
        let mut located = Vec::new();
        for scanned in self.vault.scan() {
            match scanned {
                Ok((path, memory)) => located.push((memory, path)),
                Err(_) => unreadable_files += 1,
            }
        }
        let changes = self.embedded(located)?;
        self.index.rebuild(&changes)?;
        self.index.register_external_links(&self.state.external_links()?)?;
        self.health.lock().unwrap_or_else(PoisonError::into_inner).needs_rebuild = false;
        let took_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        Ok(RebuildReport { index: self.index_report(), unreadable_files, took_ms })
    }

    pub(crate) fn heal_index(&self) {
        let health = std::mem::take(&mut *self.health.lock().unwrap_or_else(PoisonError::into_inner));
        if health.needs_rebuild || health.missing_vectors > 0 {
            match self.rebuild_index() {
                Ok(report) => tracing::info!(
                    "index rebuilt from the files: {} facts, {} entities",
                    report.index.facts,
                    report.index.entities
                ),
                Err(error) => tracing::error!("rebuilding the index failed: {error}"),
            }
        }
    }

    pub(crate) fn advance_model_swap(&self) {
        let failure = self.reembed_batch().err().map(|error| error.to_string());
        let mut last = self.model_swap_failure.lock().unwrap_or_else(PoisonError::into_inner);
        if failure == *last {
            return;
        }
        if let Some(error) = &failure {
            tracing::error!("re-embedding a model swap failed: {error}");
        } else {
            tracing::info!("re-embedding a model swap works again");
        }
        *last = failure;
    }

    fn reembed_batch(&self) -> Result<(), IndexingError> {
        self.index.finish_activation()?;
        let target = self.models.embedder()?;
        let target_id = target.model_id();
        let snapshot = self.index.snapshot();
        let mut retained = vec![target_id.clone()];
        retained.extend(snapshot.model().filter(|active| **active != target_id).cloned());
        self.models.retain_embedders(&retained);
        if snapshot.model() == Some(&target_id) {
            return Ok(());
        }
        let _writing = self.lock_writer();
        let mut batch = self.index.pending_vectors(&target.model_id(), EMBED_BATCH)?;
        if batch.is_empty() {
            self.index.activate_model(&target_id, target.dims())?;
            self.models.retain_embedders(std::slice::from_ref(&target_id));
            return Ok(());
        }
        let texts: Vec<String> = batch.iter().map(|change| change.memory.embedding_text()).collect();
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        let vectors = embed_documents(&target, &borrowed)?;
        for (change, vector) in batch.iter_mut().zip(vectors) {
            change.vector = Some(vector);
        }
        self.index.stage_vectors(&target.model_id(), &batch)?;
        Ok(())
    }

    pub(crate) fn index_report(&self) -> IndexReport {
        let snapshot = self.index.snapshot();
        let counts = snapshot.counts();
        IndexReport {
            facts: counts.facts,
            entities: counts.entities,
            links: counts.links,
            names: counts.names,
            vectors: counts.vectors,
            model: snapshot.model().map(ToString::to_string),
        }
    }

    pub(crate) fn embedded(&self, located: Vec<(Memory, PathBuf)>) -> Result<Vec<Change>, IndexingError> {
        let snapshot = self.index.snapshot();
        let embedder = snapshot.model().map(|model| self.models.embedder_for(model)).transpose()?;
        let Some(embedder) = &embedder else {
            return Ok(located
                .into_iter()
                .map(|(memory, path)| Change { memory, path, vector: None })
                .collect());
        };
        let mut changes = Vec::with_capacity(located.len());
        for batch in located.chunks(EMBED_BATCH) {
            let texts: Vec<String> = batch.iter().map(|(memory, _)| memory.embedding_text()).collect();
            let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
            let vectors = embed_documents(embedder, &borrowed)?;
            changes.extend(batch.iter().cloned().zip(vectors).map(|((memory, path), vector)| Change {
                memory,
                path,
                vector: Some(vector),
            }));
        }
        Ok(changes)
    }
}

fn embed_documents(
    embedder: &Arc<dyn Embedder>,
    texts: &[&str],
) -> Result<Vec<neomem_core::Vector>, ModelError> {
    let vectors = embedder.embed(texts, Purpose::Document)?;
    if vectors.len() == texts.len() && vectors.iter().all(|vector| vector.dims() == embedder.dims()) {
        Ok(vectors)
    } else {
        Err(ModelError::InvalidOutput(format!(
            "asked for {} vectors of {} dimensions, got dimensions {:?}",
            texts.len(),
            embedder.dims(),
            vectors.iter().map(neomem_core::Vector::dims).collect::<Vec<_>>()
        )))
    }
}
