use std::collections::HashMap;

use neomem_core::{Durability, EntityId, EpochMs, Importance, IntakeId, Label};
use serde::Deserialize;

use crate::extraction_prompt::ExtractionInput;

#[derive(Debug, Deserialize)]
pub(crate) struct ExtractionOutput {
    facts: Vec<OutputFact>,
    entities: Vec<OutputEntity>,
}

#[derive(Debug, Deserialize)]
struct OutputFact {
    content: String,
    sources: Vec<i64>,
    about: Vec<String>,
    valid_from: Option<String>,
    importance: i64,
    durability: Durability,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OutputEntity {
    #[serde(rename = "ref")]
    reference: String,
    name: String,
    kind: String,
    aliases: Vec<String>,
    existing: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EntityTarget {
    Known(EntityId),
    Named { name: String, kind: Label, aliases: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtractedFact {
    pub(crate) content: String,
    pub(crate) sources: Vec<IntakeId>,
    pub(crate) about: Vec<EntityTarget>,
    pub(crate) valid_from: Option<EpochMs>,
    pub(crate) importance: Importance,
    pub(crate) durability: Durability,
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Extracted {
    pub(crate) facts: Vec<ExtractedFact>,
    pub(crate) rejected: usize,
}

impl ExtractionOutput {
    pub(crate) fn resolve(self, input: &ExtractionInput) -> Extracted {
        let targets: HashMap<String, EntityTarget> = self
            .entities
            .into_iter()
            .map(|entity| (entity.reference.clone(), target_for(entity, input)))
            .collect();
        let mut extracted = Extracted::default();
        for fact in self.facts {
            match resolve_fact(fact, input, &targets) {
                Some(fact) => extracted.facts.push(fact),
                None => extracted.rejected += 1,
            }
        }
        extracted
    }
}

fn target_for(entity: OutputEntity, input: &ExtractionInput) -> EntityTarget {
    let known = entity
        .existing
        .and_then(|number| usize::try_from(number).ok())
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| input.entities.get(index));
    match known {
        Some(known) => EntityTarget::Known(known.id()),
        None => EntityTarget::Named {
            name: entity.name.trim().to_owned(),
            kind: Label::new(&entity.kind).unwrap_or_else(|_| Label::thing()),
            aliases: entity.aliases,
        },
    }
}

fn resolve_fact(
    fact: OutputFact,
    input: &ExtractionInput,
    targets: &HashMap<String, EntityTarget>,
) -> Option<ExtractedFact> {
    let content = fact.content.trim().to_owned();
    let sources: Vec<IntakeId> = fact
        .sources
        .iter()
        .filter_map(|number| usize::try_from(*number).ok()?.checked_sub(1))
        .filter_map(|index| input.records.get(index).map(|logged| logged.id))
        .collect();
    let about: Vec<EntityTarget> = fact
        .about
        .iter()
        .filter_map(|reference| targets.get(reference).cloned())
        .filter(|target| !matches!(target, EntityTarget::Named { name, .. } if name.is_empty()))
        .collect();
    if content.is_empty() || sources.is_empty() || about.is_empty() {
        return None;
    }
    let importance =
        u8::try_from(fact.importance.clamp(1, 10)).ok().and_then(|value| Importance::new(value).ok())?;
    Some(ExtractedFact {
        content,
        sources,
        about,
        valid_from: fact.valid_from.as_deref().and_then(parse_local_time),
        importance,
        durability: fact.durability,
        tags: fact.tags,
    })
}

fn parse_local_time(text: &str) -> Option<EpochMs> {
    let text = text.trim();
    let civil = text.parse::<jiff::civil::DateTime>().ok().or_else(|| {
        text.parse::<jiff::civil::Date>().ok().map(|date| date.to_datetime(jiff::civil::Time::midnight()))
    })?;
    let zoned = civil.to_zoned(jiff::tz::TimeZone::system()).ok()?;
    Some(EpochMs::from_millis(zoned.timestamp().as_millisecond()))
}
