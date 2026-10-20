use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use arc_swap::ArcSwap;
use neomem_core::{FactId, Memory, MemoryId, ModelId};

use crate::IndexError;
use crate::snapshot::{IndexCounts, Snapshot, Stored};
use crate::store::{Change, IndexDb};

#[derive(Debug)]
pub struct Index {
    db: Mutex<IndexDb>,
    current: ArcSwap<Snapshot>,
    model: Option<ModelId>,
    dims: Option<usize>,
}

#[derive(Debug)]
pub struct Opened {
    pub index: Index,
    pub needs_rebuild: bool,
    pub missing_vectors: Vec<MemoryId>,
}

impl Index {
    pub fn open(path: &Path, model: Option<(ModelId, usize)>) -> Result<Opened, IndexError> {
        let (db, fresh) = IndexDb::open(path)?;
        let (model, dims) = model.map_or((None, None), |(model, dims)| (Some(model), Some(dims)));
        let mut snapshot = Snapshot::empty(model.clone(), dims)?;
        let mut missing_vectors = Vec::new();
        for row in db.load(model.as_ref())? {
            let id = row.memory.id();
            if row.vector.is_none() && dims.is_some() {
                missing_vectors.push(id);
            }
            insert(&mut snapshot, row.memory, row.vector.as_ref())?;
        }
        snapshot.rederive();
        let index = Self { db: Mutex::new(db), current: ArcSwap::from_pointee(snapshot), model, dims };
        Ok(Opened { index, needs_rebuild: fresh, missing_vectors })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }

    pub fn counts(&self) -> IndexCounts {
        self.current.load().counts()
    }

    pub fn apply(&self, changes: &[Change]) -> Result<(), IndexError> {
        let mut db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        db.write(changes, self.model.as_ref(), false)?;
        let previous = self.current.load_full();
        let mut next = Snapshot::clone(&previous);
        for change in changes {
            insert(&mut next, change.memory.clone(), change.vector.as_ref())?;
        }
        next.rederive();
        db.record_latest(&moved_heads(&previous, &next))?;
        self.current.store(Arc::new(next));
        Ok(())
    }

    pub fn rebuild(&self, changes: &[Change]) -> Result<IndexCounts, IndexError> {
        let mut db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        db.write(changes, self.model.as_ref(), true)?;
        let mut next = Snapshot::empty(self.model.clone(), self.dims)?;
        for change in changes {
            insert(&mut next, change.memory.clone(), change.vector.as_ref())?;
        }
        next.rederive();
        db.record_latest(&moved_heads(&Snapshot::empty(None, None)?, &next))?;
        let counts = next.counts();
        self.current.store(Arc::new(next));
        Ok(counts)
    }
}

fn insert(
    snapshot: &mut Snapshot,
    memory: Memory,
    vector: Option<&neomem_core::Vector>,
) -> Result<(), IndexError> {
    match memory {
        Memory::Fact(fact) => {
            let id = fact.id();
            let slot = match (vector, &snapshot.fact_vectors) {
                (Some(vector), Some(vectors)) => Some(vectors.add(vector)?),
                _ => snapshot.facts.get(&id).and_then(|stored| stored.slot),
            };
            snapshot.facts.insert(id, Stored { memory: Arc::new(fact), slot });
        }
        Memory::Entity(entity) => {
            let id = entity.id();
            let slot = match (vector, &snapshot.entity_vectors) {
                (Some(vector), Some(vectors)) => Some(vectors.add(vector)?),
                _ => snapshot.entities.get(&id).and_then(|stored| stored.slot),
            };
            snapshot.entities.insert(id, Stored { memory: Arc::new(entity), slot });
        }
    }
    Ok(())
}

fn moved_heads(previous: &Snapshot, next: &Snapshot) -> Vec<(FactId, FactId)> {
    let before: HashMap<FactId, FactId> =
        previous.facts().map(|fact| (fact.id(), previous.latest(fact.id()))).collect();
    next.facts()
        .map(|fact| (fact.id(), next.latest(fact.id())))
        .filter(|(id, head)| before.get(id) != Some(head))
        .collect()
}
