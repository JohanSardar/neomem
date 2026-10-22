use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use neomem_core::{
    Entity, EntityId, EpochMs, Fact, FactLink, FactLinkKind, Filter, LlmRole, LoggedIntake, Memory,
    ModelError, NewEntity, NewFact, Purpose, Tag,
};
use neomem_index::Snapshot;
use neomem_state::{ClaimedBatch, JobId, JobOutcome, RunRecord, StateError};

use crate::Engine;
use crate::check::Settlement;
use crate::extraction_output::{EntityTarget, ExtractedFact, ExtractionOutput};
use crate::extraction_prompt::ExtractionInput;
use crate::indexing::IndexingError;
use crate::worker_config::millis;
use crate::writer::WriterGuard;

const EARLIER_TURNS: usize = 6;
const RELATED_PER_RECORD: usize = 5;
const MOST_RELATED: usize = 15;
const SAME_ENTITY_SIMILARITY: f32 = 0.92;
const GIVE_UP_AFTER: u32 = 3;
const RETRY_AFTER_MS: i64 = 60_000;
const MOST_CHARACTERS_PER_CALL: usize = 24_000;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ExtractError {
    #[error(transparent)]
    Contract(#[from] crate::contract::ContractError),
    #[error("the extractor failed: {0}")]
    Model(#[from] ModelError),
    #[error("the extractor's answer is not the expected JSON: {0}")]
    Answer(#[from] serde_json::Error),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Writing(#[from] IndexingError),
    #[error("a memory failed its own rules: {0}")]
    Memory(String),
}

#[derive(Debug, Default)]
struct BatchReport {
    records: usize,
    facts_written: usize,
    sources_attached: usize,
    linked: usize,
    unclear: usize,
    rejected: usize,
    entities_created: usize,
}

impl Engine {
    pub(crate) fn work_once(&self) -> bool {
        let now = self.clock.now();
        if let Err(error) = self.state.reclaim_expired_leases(now) {
            tracing::error!("reclaiming extraction leases failed: {error}");
        }
        let lease_until = now.plus_millis(millis(self.worker.lease));
        let claimed = match self.state.claim_ready_batch(now, lease_until, MOST_CHARACTERS_PER_CALL) {
            Ok(Some(claimed)) => claimed,
            Ok(None) => return false,
            Err(error) => {
                tracing::error!("claiming an extraction batch failed: {error}");
                return false;
            }
        };
        let jobs: Vec<JobId> = claimed.jobs.iter().map(|(job, _)| *job).collect();
        let outcome = match self.extract_batch(&claimed) {
            Ok(()) => JobOutcome::Done,
            Err(error) => self.retry_or_give_up(&claimed, &jobs, &error),
        };
        if let Err(error) = self.state.finish_jobs(&jobs, outcome, self.clock.now()) {
            tracing::error!("finishing extraction jobs failed: {error}");
        }
        true
    }

    pub(crate) fn reextract(
        &self,
        source: Option<&neomem_core::Source>,
        from: Option<EpochMs>,
    ) -> neomem_proto::Response {
        match self.state.requeue_extraction(source, from, self.clock.now()) {
            Ok(queued) => neomem_proto::Response::Reextracting { queued },
            Err(error) => crate::reply::failed(
                neomem_proto::FailureKind::Internal,
                format!("queueing re-extraction failed: {error}"),
            ),
        }
    }

    fn retry_or_give_up(&self, claimed: &ClaimedBatch, jobs: &[JobId], error: &ExtractError) -> JobOutcome {
        let attempts = jobs.first().and_then(|job| self.state.attempts(*job).ok()).unwrap_or(GIVE_UP_AFTER);
        tracing::warn!("extracting batch {} failed (attempt {attempts}): {error}", claimed.batch);
        if attempts >= GIVE_UP_AFTER {
            JobOutcome::GaveUp
        } else {
            JobOutcome::RetryAfter(self.clock.now().plus_millis(RETRY_AFTER_MS))
        }
    }

    fn extract_batch(&self, claimed: &ClaimedBatch) -> Result<(), ExtractError> {
        let started_at = self.clock.now();
        let mut records = Vec::new();
        for (_, intake) in &claimed.jobs {
            if let Some(logged) = self.state.intake(*intake)?
                && !self.state.is_corrected(*intake)?
            {
                records.push(logged);
            }
        }
        let Some(first) = records.first() else { return Ok(()) };
        let llm = self.models.llm(LlmRole::Extractor, Some(&first.record.source))?;
        let snapshot = self.index.snapshot();
        let input = self.extraction_input(records, &snapshot)?;
        let reply = llm.complete(&input.request(), &mut |_| {})?;
        let extracted =
            serde_json::from_str::<ExtractionOutput>(without_fences(&reply.text))?.resolve(&input);
        let writing = self.lock_writer();
        let mut report = BatchReport {
            records: input.records.len(),
            rejected: extracted.rejected,
            ..BatchReport::default()
        };
        let mut created = HashMap::new();
        for fact in extracted.facts {
            self.settle_and_write(&fact, &input.records, &mut created, &mut report, &writing)?;
        }
        drop(writing);
        self.log_extraction(started_at, &llm.model_id().to_string(), &report);
        Ok(())
    }

    fn extraction_input(
        &self,
        records: Vec<LoggedIntake>,
        snapshot: &Snapshot,
    ) -> Result<ExtractionInput, ExtractError> {
        let earlier = match records
            .first()
            .and_then(|first| first.record.session.as_deref().map(|session| (first, session)))
        {
            Some((first, session)) => self.state.session_turns_before(
                &first.record.source,
                session,
                first.recorded_at,
                EARLIER_TURNS,
            )?,
            None => Vec::new(),
        };
        let (facts, entities) = self.related_memories(&records, snapshot);
        Ok(ExtractionInput {
            records,
            earlier,
            entities,
            facts,
            tags: self.state.tags()?,
            today: self.clock.now(),
            owner: self.worker.owner.clone(),
        })
    }

    fn related_memories(
        &self,
        records: &[LoggedIntake],
        snapshot: &Snapshot,
    ) -> (Vec<Arc<Fact>>, Vec<Arc<Entity>>) {
        let mut facts: Vec<Arc<Fact>> = Vec::new();
        let mut entities: Vec<Arc<Entity>> = Vec::new();
        let mut add_entity = |entity: &Arc<Entity>| {
            if !entities.iter().any(|known| known.id() == entity.id()) && entities.len() < MOST_RELATED {
                entities.push(Arc::clone(entity));
            }
        };
        for hint in records.iter().flat_map(|logged| logged.record.entity_hints.iter()) {
            for id in snapshot.resolve(hint) {
                if let Some(entity) = snapshot.entity(id) {
                    add_entity(entity);
                }
            }
        }
        let texts: Vec<&str> = records.iter().map(|logged| logged.record.content.as_str()).collect();
        let vectors = snapshot
            .model()
            .and_then(|model| self.models.embedder_for(model).ok())
            .and_then(|embedder| embedder.embed(&texts, Purpose::Query).ok())
            .unwrap_or_default();
        for vector in &vectors {
            for (id, _) in snapshot.nearest_entities(vector, RELATED_PER_RECORD) {
                if let Some(entity) = snapshot.entity(id) {
                    add_entity(entity);
                }
            }
            for (seed, _) in snapshot.nearest_facts(vector, &Filter::default(), RELATED_PER_RECORD) {
                let head = snapshot.latest(seed);
                if let Some(fact) = snapshot.fact(head).filter(|fact| fact.invalid_at().is_none())
                    && !facts.iter().any(|known| known.id() == head)
                    && facts.len() < MOST_RELATED
                {
                    facts.push(Arc::clone(fact));
                }
            }
        }
        (facts, entities)
    }

    fn settle_and_write(
        &self,
        extracted: &ExtractedFact,
        records: &[LoggedIntake],
        created: &mut HashMap<String, EntityId>,
        report: &mut BatchReport,
        writing: &WriterGuard<'_>,
    ) -> Result<(), ExtractError> {
        let now = self.clock.now();
        let mut about = Vec::new();
        for target in &extracted.about {
            about.push(self.entity_for(target, created, report, writing)?);
        }
        about.dedup();
        let snapshot = self.index.snapshot();
        let mut new_fact = self.new_fact(extracted, about, records, now)?;
        let settlement = match self.embed_one(&snapshot, new_fact.content()) {
            Some(vector) => self.settle(&snapshot, new_fact.content(), &vector),
            None => Settlement::Unclear,
        };
        let mut writes = Vec::new();
        match settlement {
            Settlement::Same(existing) => {
                let Some(existing) = snapshot.fact(existing) else { return Ok(()) };
                let mut existing = Fact::clone(existing);
                for source in extracted.sources.iter().copied() {
                    if existing.attach_source(source, now) {
                        report.sources_attached += 1;
                    }
                }
                writes.push(Memory::Fact(existing));
            }
            Settlement::Linked(kind, other) => {
                new_fact
                    .link(FactLink { kind, to: other }, now)
                    .map_err(|error| ExtractError::Memory(error.to_string()))?;
                if kind == FactLinkKind::Updates
                    && let Some(old) = snapshot.fact(other)
                {
                    let mut old = Fact::clone(old);
                    old.invalidate(new_fact.valid_at(), now);
                    writes.push(Memory::Fact(old));
                }
                report.linked += 1;
                report.facts_written += 1;
                writes.insert(0, Memory::Fact(new_fact));
            }
            Settlement::New | Settlement::Unclear => {
                if settlement == Settlement::Unclear {
                    report.unclear += 1;
                }
                report.facts_written += 1;
                writes.push(Memory::Fact(new_fact));
            }
        }
        self.write_memories(writes, writing)?;
        Ok(())
    }

    fn new_fact(
        &self,
        extracted: &ExtractedFact,
        about: Vec<EntityId>,
        records: &[LoggedIntake],
        now: EpochMs,
    ) -> Result<Fact, ExtractError> {
        let cited: Vec<&LoggedIntake> =
            records.iter().filter(|logged| extracted.sources.contains(&logged.id)).collect();
        let valid_at = extracted
            .valid_from
            .or_else(|| cited.iter().map(|logged| logged.record.occurred.start).min())
            .unwrap_or(now);
        let proposed: BTreeSet<&str> = cited
            .iter()
            .flat_map(|logged| logged.record.tags.iter().map(Tag::as_str))
            .chain(extracted.tags.iter().map(String::as_str))
            .collect();
        let mut tags = BTreeSet::new();
        for tag in proposed {
            tags.insert(self.canonical_tag(tag)?);
        }
        Fact::new(NewFact {
            id: self.ids.fact(now),
            content: extracted.content.clone(),
            tags: tags.into_iter().collect(),
            about,
            recorded_at: now,
            valid_at,
            importance: extracted.importance,
            durability: extracted.durability,
            sources: extracted.sources.clone(),
            links: Vec::new(),
        })
        .map_err(|error| ExtractError::Memory(error.to_string()))
    }

    fn entity_for(
        &self,
        target: &EntityTarget,
        created: &mut HashMap<String, EntityId>,
        report: &mut BatchReport,
        writing: &WriterGuard<'_>,
    ) -> Result<EntityId, ExtractError> {
        let snapshot = self.index.snapshot();
        let (name, kind, aliases) = match target {
            EntityTarget::Known(id) => return Ok(snapshot.survivor(*id)),
            EntityTarget::Named { name, kind, aliases } => (name, kind, aliases),
        };
        let key = name.to_lowercase();
        if let Some(id) = created.get(&key) {
            return Ok(*id);
        }
        let by_name = std::iter::once(name)
            .chain(aliases.iter())
            .find_map(|candidate| snapshot.resolve(candidate).first().copied());
        let by_meaning = || {
            let vector = self.embed_one(&snapshot, &format!("{name}, a {kind}"))?;
            snapshot
                .nearest_entities(&vector, 1)
                .into_iter()
                .find(|(_, similarity)| *similarity >= SAME_ENTITY_SIMILARITY)
                .map(|(id, _)| id)
        };
        if let Some(id) = by_name.or_else(by_meaning) {
            created.insert(key, id);
            return Ok(id);
        }
        let now = self.clock.now();
        let entity = Entity::new(NewEntity {
            id: self.ids.entity(now),
            kind: kind.clone(),
            name: name.clone(),
            aliases: aliases
                .iter()
                .filter(|alias| !alias.trim().is_empty() && !alias.eq_ignore_ascii_case(name))
                .cloned()
                .collect(),
            description: String::new(),
            recorded_at: now,
        })
        .map_err(|error| ExtractError::Memory(error.to_string()))?;
        let id = entity.id();
        self.write_memories(vec![Memory::Entity(entity)], writing)?;
        created.insert(key, id);
        report.entities_created += 1;
        Ok(id)
    }

    fn log_extraction(&self, started_at: EpochMs, model: &str, report: &BatchReport) {
        let run = RunRecord {
            kind: "extract",
            started_at,
            finished_at: self.clock.now(),
            model: Some(model),
            changes: serde_json::json!({
                "records": report.records,
                "facts_written": report.facts_written,
                "sources_attached": report.sources_attached,
                "linked": report.linked,
                "unclear": report.unclear,
                "rejected": report.rejected,
                "entities_created": report.entities_created,
            }),
        };
        if let Err(error) = self.state.log_run(&run) {
            tracing::error!("recording the extraction run failed: {error}");
        }
    }
}

fn without_fences(text: &str) -> &str {
    let trimmed = text.trim();
    let opened = trimmed.strip_prefix("```json").or_else(|| trimmed.strip_prefix("```")).unwrap_or(trimmed);
    opened.strip_suffix("```").unwrap_or(opened).trim()
}
