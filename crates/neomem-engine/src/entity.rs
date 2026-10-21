use std::fs;

use neomem_core::{Entity, EntityId, Fact, MemoryId};
use neomem_index::Snapshot;
use neomem_proto::{
    EntityView, FailureKind, HistoryView, LinkedEntity, ResolvedEntity, Response, ShownMemory, SourceView,
};

use crate::Engine;
use crate::reply::failed;

impl Engine {
    pub(crate) fn resolve(&self, name: &str) -> Response {
        let snapshot = self.index.snapshot();
        let found = resolved_ids(&snapshot, name);
        if found.is_empty() {
            return failed(FailureKind::NotFound, format!("no entity is called {name}"));
        }
        Response::Resolved(
            found
                .iter()
                .filter_map(|id| snapshot.entity(*id))
                .map(|entity| ResolvedEntity {
                    id: entity.id(),
                    name: entity.name().to_owned(),
                    kind: entity.kind().clone(),
                })
                .collect(),
        )
    }

    pub(crate) fn entity(&self, name: &str) -> Response {
        let snapshot = self.index.snapshot();
        let Some(entity) = resolved_ids(&snapshot, name).first().and_then(|id| snapshot.entity(*id)) else {
            return failed(FailureKind::NotFound, format!("no entity is called {name}"));
        };
        Response::Entity(Box::new(EntityView {
            entity: Entity::clone(entity),
            facts: current_facts_about(&snapshot, entity.id()),
            linked: linked_entities(&snapshot, entity.id()),
        }))
    }

    pub(crate) fn show(&self, id: MemoryId) -> Response {
        let Some(path) = self.vault.path_of(id) else {
            return failed(FailureKind::NotFound, format!("there is no memory {id}"));
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => return failed(FailureKind::Internal, format!("reading {id} failed: {error}")),
        };
        let sources = match id {
            MemoryId::Fact(fact) => {
                self.index.snapshot().fact(fact).map(|fact| self.source_views(fact)).unwrap_or_default()
            }
            MemoryId::Entity(_) => Vec::new(),
        };
        Response::Shown(Box::new(ShownMemory { id, path: path.display().to_string(), text, sources }))
    }

    pub(crate) fn history(&self, id: neomem_core::FactId) -> Response {
        let snapshot = self.index.snapshot();
        if snapshot.fact(id).is_none() {
            return failed(FailureKind::NotFound, format!("there is no fact {id}"));
        }
        let members = snapshot.chain(id);
        let versions: Vec<Fact> = members
            .iter()
            .filter_map(|member| snapshot.fact(*member))
            .map(|fact| Fact::clone(fact))
            .collect();
        let extensions: Vec<Fact> = members
            .iter()
            .flat_map(|member| snapshot.neighbors(MemoryId::Fact(*member)).iter())
            .filter(|neighbor| neighbor.kind == neomem_core::LinkKind::Extends)
            .filter_map(|neighbor| match neighbor.id {
                MemoryId::Fact(fact) if !members.contains(&fact) => {
                    snapshot.fact(fact).map(|found| Fact::clone(found))
                }
                _ => None,
            })
            .collect();
        Response::History(Box::new(HistoryView { versions, current: snapshot.latest(id), extensions }))
    }

    pub(crate) fn mark_useful(&self, id: neomem_core::FactId) -> Response {
        let writing = self.lock_writer();
        let Some(fact) = self.index.snapshot().fact(id).map(|fact| Fact::clone(fact)) else {
            return failed(FailureKind::NotFound, format!("there is no fact {id}"));
        };
        let mut fact = fact;
        fact.mark_useful(self.clock.now());
        let evidence = fact.evidence().count();
        match self.write_memories(vec![neomem_core::Memory::Fact(fact)], &writing) {
            Ok(()) => Response::MarkedUseful { id, evidence },
            Err(error) => failed(FailureKind::Internal, format!("recording the evidence failed: {error}")),
        }
    }

    fn source_views(&self, fact: &Fact) -> Vec<SourceView> {
        fact.sources()
            .iter()
            .map(|source| SourceView {
                id: *source,
                record: self.state.intake(*source).ok().flatten().map(|logged| logged.record),
                corrected: self.state.is_corrected(*source).unwrap_or(false),
            })
            .collect()
    }
}

fn resolved_ids(snapshot: &Snapshot, name: &str) -> Vec<EntityId> {
    match name.trim().parse::<EntityId>() {
        Ok(id) if snapshot.entity(id).is_some() => vec![snapshot.survivor(id)],
        _ => snapshot.resolve(name),
    }
}

fn current_facts_about(snapshot: &Snapshot, entity: EntityId) -> Vec<Fact> {
    let mut facts: Vec<Fact> = snapshot
        .entities()
        .filter(|candidate| snapshot.survivor(candidate.id()) == entity)
        .flat_map(|candidate| snapshot.facts_about(candidate.id()).iter().copied())
        .filter(|id| snapshot.latest(*id) == *id)
        .filter_map(|id| snapshot.fact(id))
        .filter(|fact| fact.invalid_at().is_none())
        .map(|fact| Fact::clone(fact))
        .collect();
    facts.sort_by_key(|fact| std::cmp::Reverse((fact.valid_at(), fact.recorded_at())));
    facts.dedup_by_key(|fact| fact.id());
    facts
}

fn linked_entities(snapshot: &Snapshot, entity: EntityId) -> Vec<LinkedEntity> {
    snapshot
        .neighbors(MemoryId::Entity(entity))
        .iter()
        .filter_map(|neighbor| match neighbor.id {
            MemoryId::Entity(other) => snapshot.entity(other).map(|found| LinkedEntity {
                kind: neighbor.kind.clone(),
                direction: neighbor.direction,
                entity: Entity::clone(found),
            }),
            MemoryId::Fact(_) => None,
        })
        .collect()
}
