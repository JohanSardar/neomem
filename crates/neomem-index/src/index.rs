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
}

#[derive(Debug)]
pub struct Opened {
    pub index: Index,
    pub needs_rebuild: bool,
    pub missing_vectors: Vec<MemoryId>,
}

impl Index {
    pub fn register_external_links(&self, links: &[neomem_core::ExternalLink]) -> Result<(), IndexError> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner).register_external_links(links)
    }

    pub fn open(path: &Path, model: Option<(ModelId, usize)>) -> Result<Opened, IndexError> {
        let (mut db, fresh) = IndexDb::open(path)?;
        let model = db.active_model()?.or(model);
        if db.active_model()?.is_none()
            && let Some((model, dims)) = &model
        {
            db.activate(model, *dims)?;
        }
        db.finish_activation()?;
        let (model, dims) = model.map_or((None, None), |(model, dims)| (Some(model), Some(dims)));
        let mut snapshot = Snapshot::empty(model.clone(), dims)?;
        let rows = db.load(model.as_ref())?;
        let missing_vectors = rows
            .iter()
            .filter(|row| row.vector.is_none() && dims.is_some())
            .map(|row| row.memory.id())
            .collect();
        insert_all(&mut snapshot, rows.into_iter().map(|row| (row.memory, row.vector)))?;
        snapshot.rederive();
        let index = Self { db: Mutex::new(db), current: ArcSwap::from_pointee(snapshot) };
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
        let previous = self.current.load_full();
        db.write(changes, previous.model(), false)?;
        let mut next = Snapshot::clone(&previous);
        insert_all(&mut next, changes.iter().map(|change| (change.memory.clone(), change.vector.clone())))?;
        next.rederive();
        db.record_latest(&moved_heads(&previous, &next))?;
        self.current.store(Arc::new(next));
        Ok(())
    }

    pub fn rebuild(&self, changes: &[Change]) -> Result<IndexCounts, IndexError> {
        let mut db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = self.current.load_full();
        db.write(changes, previous.model(), true)?;
        let mut next = Snapshot::empty(previous.model().cloned(), previous.dims())?;
        insert_all(&mut next, changes.iter().map(|change| (change.memory.clone(), change.vector.clone())))?;
        next.rederive();
        db.record_latest(&moved_heads(&Snapshot::empty(None, None)?, &next))?;
        let counts = next.counts();
        self.current.store(Arc::new(next));
        Ok(counts)
    }
    pub fn pending_vectors(&self, model: &ModelId, limit: usize) -> Result<Vec<Change>, IndexError> {
        let db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(db
            .missing(model, limit)?
            .into_iter()
            .map(|row| Change { memory: row.memory, path: row.path, vector: None })
            .collect())
    }

    pub fn stage_vectors(&self, model: &ModelId, changes: &[Change]) -> Result<(), IndexError> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner).stage_vectors(model, changes)
    }

    pub fn activate_model(&self, model: &ModelId, dims: usize) -> Result<(), IndexError> {
        let mut db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = Snapshot::empty(Some(model.clone()), Some(dims))?;
        let rows = db.load(Some(model))?;
        if rows.iter().any(|row| row.vector.is_none()) {
            return Err(IndexError::Corrupt("cannot activate an incomplete vector set".to_owned()));
        }
        insert_all(&mut next, rows.into_iter().map(|row| (row.memory, row.vector)))?;
        next.rederive();
        db.activate(model, dims)?;
        self.current.store(Arc::new(next));
        db.finish_activation()?;
        Ok(())
    }

    pub fn finish_activation(&self) -> Result<(), IndexError> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner).finish_activation()
    }
}

fn insert_all(
    snapshot: &mut Snapshot,
    entries: impl IntoIterator<Item = (Memory, Option<neomem_core::Vector>)>,
) -> Result<(), IndexError> {
    let entries: Vec<(Memory, Option<neomem_core::Vector>)> = entries.into_iter().collect();
    let fact_slots =
        new_slots(snapshot.fact_vectors.as_deref(), &entries, |memory| matches!(memory, Memory::Fact(_)))?;
    let entity_slots = new_slots(snapshot.entity_vectors.as_deref(), &entries, |memory| {
        matches!(memory, Memory::Entity(_))
    })?;
    let (mut fact_slots, mut entity_slots) = (fact_slots.into_iter(), entity_slots.into_iter());
    for (memory, vector) in entries {
        let embedded = vector.is_some();
        match memory {
            Memory::Fact(fact) => {
                let id = fact.id();
                let fresh = if embedded { fact_slots.next().flatten() } else { None };
                let slot = fresh.or_else(|| snapshot.facts.get(&id).and_then(|stored| stored.slot));
                snapshot.facts.insert(id, Stored { memory: Arc::new(fact), slot });
            }
            Memory::Entity(entity) => {
                let id = entity.id();
                let fresh = if embedded { entity_slots.next().flatten() } else { None };
                let slot = fresh.or_else(|| snapshot.entities.get(&id).and_then(|stored| stored.slot));
                snapshot.entities.insert(id, Stored { memory: Arc::new(entity), slot });
            }
        }
    }
    Ok(())
}

fn new_slots(
    vectors: Option<&crate::vectors::VectorIndex>,
    entries: &[(Memory, Option<neomem_core::Vector>)],
    wanted: impl Fn(&Memory) -> bool,
) -> Result<Vec<Option<crate::vectors::Slot>>, IndexError> {
    let embedded: Vec<&neomem_core::Vector> = entries
        .iter()
        .filter(|(memory, _)| wanted(memory))
        .filter_map(|(_, vector)| vector.as_ref())
        .collect();
    Ok(match vectors {
        Some(vectors) => vectors.add_all(&embedded)?.into_iter().map(Some).collect(),
        None => vec![None; embedded.len()],
    })
}

fn moved_heads(previous: &Snapshot, next: &Snapshot) -> Vec<(FactId, FactId)> {
    let before: HashMap<FactId, FactId> =
        previous.facts().map(|fact| (fact.id(), previous.latest(fact.id()))).collect();
    next.facts()
        .map(|fact| (fact.id(), next.latest(fact.id())))
        .filter(|(id, head)| before.get(id) != Some(head))
        .collect()
}
