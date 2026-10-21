use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use neomem_core::{
    EntityId, EpochMs, Fact, FactId, FactLinkKind, Filter, LinkKind, MemoryId, ModelError, Purpose, Vector,
    decay_factor, rank_score,
};
use neomem_index::Snapshot;
use neomem_proto::{
    EntityRef, FailureKind, LinkedFact, Response, SearchHit, SearchQuery, SearchResults, SearchTrace,
};

use crate::Engine;
use crate::reply::failed;

#[derive(Debug, thiserror::Error)]
enum SearchError {
    #[error("search needs an embedder and none is working")]
    NoEmbedder,
    #[error("embedding the query failed: {0}")]
    Embedding(#[from] ModelError),
    #[error("no entity is called {0}")]
    UnknownEntity(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Versions {
    Latest,
    AsOf(EpochMs),
    Everything,
}

struct Candidate {
    fact: Arc<Fact>,
    similarity: f32,
    replaced: Vec<FactId>,
}

struct Lap(Instant);

impl Lap {
    fn start() -> Self {
        Self(Instant::now())
    }

    fn next(&mut self) -> f64 {
        let elapsed = self.0.elapsed().as_secs_f64() * 1000.0;
        self.0 = Instant::now();
        elapsed
    }
}

impl Engine {
    pub(crate) fn search(&self, query: &SearchQuery) -> Response {
        match self.run_search(query) {
            Ok(results) => Response::SearchResults(Box::new(results)),
            Err(SearchError::UnknownEntity(name)) => {
                failed(FailureKind::NotFound, format!("no entity is called {name}"))
            }
            Err(error) => failed(FailureKind::Unavailable, error.to_string()),
        }
    }

    fn run_search(&self, query: &SearchQuery) -> Result<SearchResults, SearchError> {
        let started = Instant::now();
        let mut lap = Lap::start();
        let embedder = self.embedder.as_ref().ok_or(SearchError::NoEmbedder)?;
        let vector = embedder
            .embed(&[query.text.as_str()], Purpose::Query)?
            .into_iter()
            .next()
            .ok_or(SearchError::NoEmbedder)?;
        let embed_ms = lap.next();
        let snapshot = self.index.snapshot();
        let filter = filter_for(&snapshot, query)?;
        let config = &self.search_config;
        let seeds = snapshot.nearest_facts(&vector, &filter, query.k.unwrap_or(config.k).max(1));
        let seeds_ms = lap.next();
        let versions = versions_for(query);
        let candidates: Vec<Candidate> = choose_versions(&snapshot, &seeds, versions)
            .into_iter()
            .filter(|candidate| self.is_visible(&candidate.fact, versions))
            .filter(|candidate| {
                filter.accepts_validity(candidate.fact.valid_at(), candidate.fact.invalid_at())
            })
            .collect();
        let latest_ms = lap.next();
        let now = self.clock.now();
        let mut hits = Self::ranked(&snapshot, candidates, versions, now);
        flag_conflicts(&snapshot, &mut hits, config.near_duplicate);
        let rank_ms = lap.next();
        let linked_kept = self.attach_links(&snapshot, &vector, &mut hits, versions, now);
        let links_ms = lap.next();
        let trace = query.trace.then(|| SearchTrace {
            embed_ms,
            seeds_ms,
            latest_ms,
            links_ms,
            rank_ms,
            total_ms: started.elapsed().as_secs_f64() * 1000.0,
            seeds: seeds.len(),
            after_latest: hits.len(),
            linked_kept,
        });
        Ok(SearchResults { hits, model: embedder.model_id().to_string(), trace })
    }

    fn is_visible(&self, fact: &Fact, versions: Versions) -> bool {
        let shows_history = versions != Versions::Latest;
        let valid_now = fact.invalid_at().is_none();
        shows_history || (valid_now && !self.only_corrected_sources(fact))
    }

    fn only_corrected_sources(&self, fact: &Fact) -> bool {
        fact.sources().iter().all(|source| self.state.is_corrected(*source).unwrap_or(false))
    }

    fn ranked(
        snapshot: &Snapshot,
        candidates: Vec<Candidate>,
        versions: Versions,
        now: EpochMs,
    ) -> Vec<SearchHit> {
        let mut hits: Vec<SearchHit> = candidates
            .into_iter()
            .map(|candidate| {
                let fact = &candidate.fact;
                let decay = decay_for(fact, versions, now);
                let score =
                    rank_score(candidate.similarity, decay, fact.importance(), invalid_for(fact, versions));
                SearchHit {
                    fact: Fact::clone(fact),
                    score,
                    similarity: candidate.similarity,
                    decay,
                    replaced: candidate.replaced,
                    conflict: false,
                    linked: Vec::new(),
                    entities: entity_refs(snapshot, fact),
                }
            })
            .collect();
        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        hits
    }

    fn attach_links(
        &self,
        snapshot: &Snapshot,
        query: &Vector,
        hits: &mut [SearchHit],
        versions: Versions,
        now: EpochMs,
    ) -> usize {
        let config = self.search_config;
        let weakest = hits.iter().map(|hit| hit.score).fold(f32::INFINITY, f32::min);
        let shown: HashSet<FactId> = hits.iter().map(|hit| hit.fact.id()).collect();
        let mut kept = 0;
        for hit in hits.iter_mut() {
            let mut linked: Vec<LinkedFact> = snapshot
                .neighbors(MemoryId::Fact(hit.fact.id()))
                .iter()
                .filter_map(|neighbor| {
                    let weight = config.weight_of(&neighbor.kind)?;
                    let MemoryId::Fact(id) = neighbor.id else { return None };
                    let id = if versions == Versions::Latest { snapshot.latest(id) } else { id };
                    let fact = snapshot
                        .fact(id)
                        .filter(|fact| !shown.contains(&fact.id()) && self.is_visible(fact, versions))?;
                    let similarity = snapshot.fact_vector(id).map_or(0.0, |vector| vector.cosine(query));
                    let usefulness = similarity * weight * decay_for(fact, versions, now);
                    let contradiction = neighbor.kind == LinkKind::Contradicts;
                    (contradiction || usefulness >= config.keep_ratio * weakest).then(|| LinkedFact {
                        fact: Fact::clone(fact),
                        kind: neighbor.kind.clone(),
                        direction: neighbor.direction,
                        usefulness,
                    })
                })
                .collect();
            linked.sort_by(|left, right| {
                (right.kind == LinkKind::Contradicts)
                    .cmp(&(left.kind == LinkKind::Contradicts))
                    .then(right.usefulness.total_cmp(&left.usefulness))
            });
            linked.dedup_by_key(|link| link.fact.id());
            linked.truncate(config.linked_per_seed);
            if linked.iter().any(|link| link.kind == LinkKind::Contradicts) {
                hit.conflict = true;
            }
            kept += linked.len();
            hit.linked = linked;
        }
        kept
    }
}

fn versions_for(query: &SearchQuery) -> Versions {
    match (query.as_of, query.include_invalid) {
        (Some(moment), _) => Versions::AsOf(moment),
        (None, true) => Versions::Everything,
        (None, false) => Versions::Latest,
    }
}

fn invalid_for(fact: &Fact, versions: Versions) -> bool {
    match versions {
        Versions::AsOf(moment) => !fact.is_valid_at(moment),
        Versions::Latest | Versions::Everything => fact.invalid_at().is_some(),
    }
}

fn decay_for(fact: &Fact, versions: Versions, now: EpochMs) -> f32 {
    if versions == Versions::Latest {
        decay_factor(fact.evidence(), now, fact.durability(), fact.importance())
    } else {
        1.0
    }
}

fn filter_for(snapshot: &Snapshot, query: &SearchQuery) -> Result<Filter, SearchError> {
    let mut entities = Vec::new();
    for name in &query.entities {
        let named = match name.parse::<EntityId>() {
            Ok(id) if snapshot.entity(id).is_some() => vec![snapshot.survivor(id)],
            _ => snapshot.resolve(name),
        };
        if named.is_empty() {
            return Err(SearchError::UnknownEntity(name.clone()));
        }
        entities.extend(named);
    }
    let merged_into_them: Vec<EntityId> = snapshot
        .entities()
        .map(|entity| entity.id())
        .filter(|id| entities.contains(&snapshot.survivor(*id)) && !entities.contains(id))
        .collect();
    entities.extend(merged_into_them);
    Ok(Filter {
        tags: query.tags.clone(),
        entities,
        from: query.from,
        to: query.to,
        include_hidden_tags: query.include_hidden,
    })
}

fn choose_versions(snapshot: &Snapshot, seeds: &[(FactId, f32)], versions: Versions) -> Vec<Candidate> {
    let mut chosen: Vec<Candidate> = Vec::new();
    let mut positions: HashMap<FactId, usize> = HashMap::new();
    for (seed, similarity) in seeds {
        let version = match versions {
            Versions::Latest => Some(snapshot.latest(*seed)),
            Versions::Everything => Some(*seed),
            Versions::AsOf(moment) => version_valid_at(snapshot, *seed, moment),
        };
        let Some(fact) = version.and_then(|id| snapshot.fact(id)) else { continue };
        let replaced = (fact.id() != *seed).then_some(*seed);
        if let Some(position) = positions.get(&fact.id()) {
            let existing = &mut chosen[*position];
            existing.similarity = existing.similarity.max(*similarity);
            existing.replaced.extend(replaced);
        } else {
            positions.insert(fact.id(), chosen.len());
            chosen.push(Candidate {
                fact: Arc::clone(fact),
                similarity: *similarity,
                replaced: replaced.into_iter().collect(),
            });
        }
    }
    chosen
}

fn version_valid_at(snapshot: &Snapshot, seed: FactId, moment: EpochMs) -> Option<FactId> {
    snapshot.chain(seed).into_iter().rev().find(|id| {
        snapshot.fact(*id).is_some_and(|fact| {
            fact.valid_at() <= moment && fact.invalid_at().is_none_or(|ended| moment < ended)
        })
    })
}

fn entity_refs(snapshot: &Snapshot, fact: &Fact) -> Vec<EntityRef> {
    let mut refs: Vec<EntityRef> = fact
        .about()
        .iter()
        .filter_map(|id| snapshot.entity(snapshot.survivor(*id)))
        .map(|entity| EntityRef {
            id: entity.id(),
            name: entity.name().to_owned(),
            kind: entity.kind().clone(),
        })
        .collect();
    refs.dedup_by_key(|entity| entity.id);
    refs
}

fn flag_conflicts(snapshot: &Snapshot, hits: &mut [SearchHit], near_duplicate: f32) {
    let mut conflicting: Vec<(usize, usize)> = Vec::new();
    for left in 0..hits.len() {
        for right in (left + 1)..hits.len() {
            if in_conflict(snapshot, &hits[left].fact, &hits[right].fact, near_duplicate) {
                conflicting.push((left, right));
            }
        }
    }
    for (left, right) in &conflicting {
        hits[*left].conflict = true;
        hits[*right].conflict = true;
    }
    for (left, right) in conflicting {
        if newer(&hits[right].fact, &hits[left].fact) {
            hits.swap(left, right);
        }
    }
}

fn newer(candidate: &Fact, other: &Fact) -> bool {
    (candidate.valid_at(), candidate.recorded_at()) > (other.valid_at(), other.recorded_at())
}

fn in_conflict(snapshot: &Snapshot, left: &Fact, right: &Fact, near_duplicate: f32) -> bool {
    let linked = left
        .links()
        .iter()
        .any(|link| link.kind == FactLinkKind::Contradicts && link.to == right.id())
        || right.links().iter().any(|link| link.kind == FactLinkKind::Contradicts && link.to == left.id());
    if linked {
        return true;
    }
    let shares_entity = left.about().iter().any(|entity| right.about().contains(entity));
    let both_valid = left.invalid_at().is_none() && right.invalid_at().is_none();
    let different = left.content() != right.content();
    shares_entity
        && both_valid
        && different
        && snapshot
            .fact_vector(left.id())
            .zip(snapshot.fact_vector(right.id()))
            .is_some_and(|(left, right)| left.cosine(&right) >= near_duplicate)
}
