use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use neomem_core::{
    Direction, Entity, EntityId, Fact, FactId, FactLinkKind, Filter, LinkKind, MemoryId, ModelId, Vector,
};

use crate::{Slot, VectorIndex};

const SCAN_INSTEAD_BELOW: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbor {
    pub id: MemoryId,
    pub kind: LinkKind,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexCounts {
    pub facts: usize,
    pub entities: usize,
    pub links: usize,
    pub names: usize,
    pub vectors: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Stored<T> {
    pub(crate) memory: Arc<T>,
    pub(crate) slot: Option<Slot>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) facts: HashMap<FactId, Stored<Fact>>,
    pub(crate) entities: HashMap<EntityId, Stored<Entity>>,
    pub(crate) fact_vectors: Option<Arc<VectorIndex>>,
    pub(crate) entity_vectors: Option<Arc<VectorIndex>>,
    pub(crate) model: Option<ModelId>,
    fact_by_slot: HashMap<Slot, FactId>,
    entity_by_slot: HashMap<Slot, EntityId>,
    latest: HashMap<FactId, FactId>,
    links: HashMap<MemoryId, Vec<Neighbor>>,
    names: HashMap<String, Vec<EntityId>>,
    facts_about: HashMap<EntityId, Vec<FactId>>,
}

impl Snapshot {
    pub(crate) fn empty(model: Option<ModelId>, dims: Option<usize>) -> Result<Self, crate::IndexError> {
        let fresh = |dims: usize| VectorIndex::new(dims).map(Arc::new);
        Ok(Self {
            facts: HashMap::new(),
            entities: HashMap::new(),
            fact_vectors: dims.map(fresh).transpose()?,
            entity_vectors: dims.map(fresh).transpose()?,
            model,
            fact_by_slot: HashMap::new(),
            entity_by_slot: HashMap::new(),
            latest: HashMap::new(),
            links: HashMap::new(),
            names: HashMap::new(),
            facts_about: HashMap::new(),
        })
    }

    pub fn model(&self) -> Option<&ModelId> {
        self.model.as_ref()
    }

    pub fn fact(&self, id: FactId) -> Option<&Arc<Fact>> {
        self.facts.get(&id).map(|stored| &stored.memory)
    }

    pub fn entity(&self, id: EntityId) -> Option<&Arc<Entity>> {
        self.entities.get(&id).map(|stored| &stored.memory)
    }

    pub fn facts(&self) -> impl Iterator<Item = &Arc<Fact>> {
        self.facts.values().map(|stored| &stored.memory)
    }

    pub fn entities(&self) -> impl Iterator<Item = &Arc<Entity>> {
        self.entities.values().map(|stored| &stored.memory)
    }

    pub fn latest(&self, id: FactId) -> FactId {
        self.latest.get(&id).copied().unwrap_or(id)
    }

    pub fn neighbors(&self, id: MemoryId) -> &[Neighbor] {
        self.links.get(&id).map_or(&[], Vec::as_slice)
    }

    pub fn facts_about(&self, entity: EntityId) -> &[FactId] {
        self.facts_about.get(&entity).map_or(&[], Vec::as_slice)
    }

    pub fn resolve(&self, name: &str) -> Vec<EntityId> {
        let mut found: Vec<EntityId> = self
            .names
            .get(&name.trim().to_lowercase())
            .map(|ids| ids.iter().map(|id| self.survivor(*id)).collect())
            .unwrap_or_default();
        found.sort();
        found.dedup();
        found
    }

    pub fn survivor(&self, id: EntityId) -> EntityId {
        let mut current = id;
        let mut seen = HashSet::new();
        while let Some(next) = self.entity(current).and_then(|entity| entity.merged_into()) {
            if !seen.insert(current) {
                break;
            }
            current = next;
        }
        current
    }

    pub fn counts(&self) -> IndexCounts {
        IndexCounts {
            facts: self.facts.len(),
            entities: self.entities.len(),
            links: self.links.values().map(Vec::len).sum::<usize>() / 2,
            names: self.names.len(),
            vectors: self.fact_by_slot.len() + self.entity_by_slot.len(),
        }
    }

    pub fn nearest_facts(&self, query: &Vector, filter: &Filter, count: usize) -> Vec<(FactId, f32)> {
        let Some(vectors) = &self.fact_vectors else { return Vec::new() };
        let accepts = |fact: &Fact| {
            filter.accepts_tags(fact.tags())
                && filter.accepts_entities(fact.about())
                && filter.accepts_validity(fact.valid_at(), fact.invalid_at())
        };
        let candidates: Vec<FactId> = filter
            .entities
            .iter()
            .flat_map(|entity| self.facts_about(self.survivor(*entity)).iter().copied())
            .collect();
        if !filter.entities.is_empty() && candidates.len() < SCAN_INSTEAD_BELOW {
            return self.scan(vectors, query, candidates, count, accepts);
        }
        vectors
            .nearest(query, count, |slot| {
                self.fact_by_slot.get(&slot).and_then(|id| self.fact(*id)).is_some_and(|fact| accepts(fact))
            })
            .into_iter()
            .filter_map(|(slot, similarity)| self.fact_by_slot.get(&slot).map(|id| (*id, similarity)))
            .collect()
    }

    pub fn fact_vector(&self, id: FactId) -> Option<Vector> {
        let slot = self.facts.get(&id)?.slot?;
        self.fact_vectors.as_ref()?.stored(slot)
    }

    pub fn nearest_entities(&self, query: &Vector, count: usize) -> Vec<(EntityId, f32)> {
        let Some(vectors) = &self.entity_vectors else { return Vec::new() };
        vectors
            .nearest(query, count, |slot| {
                self.entity_by_slot
                    .get(&slot)
                    .and_then(|id| self.entity(*id))
                    .is_some_and(|entity| entity.merged_into().is_none())
            })
            .into_iter()
            .filter_map(|(slot, similarity)| self.entity_by_slot.get(&slot).map(|id| (*id, similarity)))
            .collect()
    }

    fn scan(
        &self,
        vectors: &VectorIndex,
        query: &Vector,
        mut candidates: Vec<FactId>,
        count: usize,
        accepts: impl Fn(&Fact) -> bool,
    ) -> Vec<(FactId, f32)> {
        candidates.sort();
        candidates.dedup();
        let mut scored: Vec<(FactId, f32)> = candidates
            .into_iter()
            .filter_map(|id| self.facts.get(&id).map(|stored| (id, stored)))
            .filter(|(_, stored)| accepts(&stored.memory))
            .filter_map(|(id, stored)| {
                stored.slot.and_then(|slot| vectors.stored(slot)).map(|vector| (id, vector.cosine(query)))
            })
            .collect();
        scored.sort_by(|left, right| right.1.total_cmp(&left.1));
        scored.truncate(count);
        scored
    }

    pub(crate) fn rederive(&mut self) {
        self.fact_by_slot =
            self.facts.iter().filter_map(|(id, stored)| stored.slot.map(|slot| (slot, *id))).collect();
        self.entity_by_slot =
            self.entities.iter().filter_map(|(id, stored)| stored.slot.map(|slot| (slot, *id))).collect();
        self.latest = latest_versions(&self.facts);
        self.links = link_map(&self.facts, &self.entities);
        self.names = name_map(&self.entities);
        self.facts_about = facts_about(&self.facts);
    }
}

fn latest_versions(facts: &HashMap<FactId, Stored<Fact>>) -> HashMap<FactId, FactId> {
    let mut successors: HashMap<FactId, Vec<FactId>> = HashMap::new();
    for stored in facts.values() {
        for link in stored.memory.links().iter().filter(|link| link.kind == FactLinkKind::Updates) {
            successors.entry(link.to).or_default().push(stored.memory.id());
        }
    }
    let newest = |candidates: &[FactId]| {
        candidates
            .iter()
            .copied()
            .filter_map(|id| {
                facts.get(&id).map(|stored| (id, stored.memory.valid_at(), stored.memory.recorded_at()))
            })
            .max_by_key(|(id, valid_at, recorded_at)| (*valid_at, *recorded_at, *id))
            .map(|(id, _, _)| id)
    };
    let mut latest = HashMap::new();
    for id in successors.keys() {
        let mut head = *id;
        let mut seen = HashSet::from([head]);
        while let Some(next) = successors.get(&head).and_then(|candidates| newest(candidates)) {
            if !seen.insert(next) {
                break;
            }
            head = next;
        }
        if head != *id {
            latest.insert(*id, head);
        }
    }
    latest
}

fn link_map(
    facts: &HashMap<FactId, Stored<Fact>>,
    entities: &HashMap<EntityId, Stored<Entity>>,
) -> HashMap<MemoryId, Vec<Neighbor>> {
    let mut links: HashMap<MemoryId, Vec<Neighbor>> = HashMap::new();
    let mut connect = |from: MemoryId, to: MemoryId, kind: LinkKind| {
        links.entry(from).or_default().push(Neighbor {
            id: to,
            kind: kind.clone(),
            direction: Direction::Outgoing,
        });
        links.entry(to).or_default().push(Neighbor { id: from, kind, direction: Direction::Incoming });
    };
    for stored in facts.values() {
        let fact = &stored.memory;
        for link in fact.links() {
            connect(MemoryId::Fact(fact.id()), MemoryId::Fact(link.to), link.kind.into());
        }
        for entity in fact.about() {
            connect(MemoryId::Fact(fact.id()), MemoryId::Entity(*entity), LinkKind::About);
        }
    }
    for stored in entities.values() {
        let entity = &stored.memory;
        for link in entity.links() {
            connect(MemoryId::Entity(entity.id()), MemoryId::Entity(link.to), link.kind.into());
        }
        for relation in entity.relations().iter().filter(|relation| relation.invalid_at.is_none()) {
            connect(
                MemoryId::Entity(entity.id()),
                MemoryId::Entity(relation.to),
                LinkKind::Relation(relation.label.clone()),
            );
        }
    }
    links
}

fn name_map(entities: &HashMap<EntityId, Stored<Entity>>) -> HashMap<String, Vec<EntityId>> {
    let mut names: HashMap<String, Vec<EntityId>> = HashMap::new();
    for stored in entities.values() {
        for name in stored.memory.names() {
            names.entry(name.trim().to_lowercase()).or_default().push(stored.memory.id());
        }
    }
    names
}

fn facts_about(facts: &HashMap<FactId, Stored<Fact>>) -> HashMap<EntityId, Vec<FactId>> {
    let mut about: HashMap<EntityId, Vec<FactId>> = HashMap::new();
    for stored in facts.values() {
        for entity in stored.memory.about() {
            about.entry(*entity).or_default().push(stored.memory.id());
        }
    }
    about
}
