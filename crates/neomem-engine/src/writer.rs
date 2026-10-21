use std::sync::{MutexGuard, PoisonError};

use neomem_core::Memory;

use crate::Engine;
use crate::indexing::IndexingError;

pub(crate) type WriterGuard<'lock> = MutexGuard<'lock, ()>;

impl Engine {
    pub(crate) fn lock_writer(&self) -> WriterGuard<'_> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn write_memories(
        &self,
        memories: Vec<Memory>,
        _writing: &WriterGuard<'_>,
    ) -> Result<(), IndexingError> {
        let now = self.clock.now();
        let mut located = Vec::with_capacity(memories.len());
        for memory in memories {
            let path = self.vault.put(&memory, now)?;
            located.push((memory, path));
        }
        let changes = self.embedded(located)?;
        self.index.apply(&changes)?;
        Ok(())
    }
}
