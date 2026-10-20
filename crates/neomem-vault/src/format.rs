use neomem_core::{
    Durability, EntityId, EntityLink, EpochMs, Fact, FactId, FactLink, Importance, IntakeId, Label, Relation,
    StoredEntity, StoredFact, Tag,
};
use serde::{Deserialize, Serialize};
use serde_saphyr::FlowSeq;

use crate::{Memory, VaultError};

const FENCE: &str = "---\n";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MemoryKind {
    Fact,
    Entity,
}

#[derive(Deserialize)]
struct KindOnly {
    kind: MemoryKind,
}

#[derive(Serialize, Deserialize)]
struct FactHeader {
    id: FactId,
    kind: MemoryKind,
    tags: FlowSeq<Vec<Tag>>,
    about: FlowSeq<Vec<EntityId>>,
    recorded_at: EpochMs,
    updated_at: EpochMs,
    valid_at: EpochMs,
    invalid_at: Option<EpochMs>,
    importance: Importance,
    durability: Durability,
    sources: FlowSeq<Vec<IntakeId>>,
    #[serde(default = "no_links")]
    links: FlowSeq<Vec<FactLink>>,
    #[serde(default = "no_evidence")]
    evidence: FlowSeq<Vec<EpochMs>>,
}

#[derive(Serialize, Deserialize)]
struct EntityHeader {
    id: EntityId,
    kind: MemoryKind,
    entity_kind: Label,
    name: String,
    #[serde(default = "no_names")]
    aliases: FlowSeq<Vec<String>>,
    recorded_at: EpochMs,
    updated_at: EpochMs,
    #[serde(default = "no_entities")]
    merged_from: FlowSeq<Vec<EntityId>>,
    #[serde(default)]
    merged_into: Option<EntityId>,
    #[serde(default = "no_relations")]
    relations: FlowSeq<Vec<Relation>>,
    #[serde(default = "no_entity_links")]
    links: FlowSeq<Vec<EntityLink>>,
}

fn no_links() -> FlowSeq<Vec<FactLink>> {
    FlowSeq(Vec::new())
}

fn no_evidence() -> FlowSeq<Vec<EpochMs>> {
    FlowSeq(Vec::new())
}

fn no_names() -> FlowSeq<Vec<String>> {
    FlowSeq(Vec::new())
}

fn no_entities() -> FlowSeq<Vec<EntityId>> {
    FlowSeq(Vec::new())
}

fn no_relations() -> FlowSeq<Vec<Relation>> {
    FlowSeq(Vec::new())
}

fn no_entity_links() -> FlowSeq<Vec<EntityLink>> {
    FlowSeq(Vec::new())
}

pub fn render(memory: &Memory) -> Result<String, VaultError> {
    let (header, body) = match memory {
        Memory::Fact(fact) => (serialize(&fact_header(fact))?, fact.content()),
        Memory::Entity(entity) => (
            serialize(&EntityHeader {
                id: entity.id(),
                kind: MemoryKind::Entity,
                entity_kind: entity.kind().clone(),
                name: entity.name().to_owned(),
                aliases: FlowSeq(entity.aliases().to_vec()),
                recorded_at: entity.recorded_at(),
                updated_at: entity.updated_at(),
                merged_from: FlowSeq(entity.merged_from().to_vec()),
                merged_into: entity.merged_into(),
                relations: FlowSeq(entity.relations().to_vec()),
                links: FlowSeq(entity.links().to_vec()),
            })?,
            entity.description(),
        ),
    };
    let body = body.trim_end();
    let separator = if body.is_empty() { "" } else { "\n" };
    Ok(format!("{FENCE}{header}{FENCE}{body}{separator}"))
}

fn fact_header(fact: &Fact) -> FactHeader {
    FactHeader {
        id: fact.id(),
        kind: MemoryKind::Fact,
        tags: FlowSeq(fact.tags().to_vec()),
        about: FlowSeq(fact.about().to_vec()),
        recorded_at: fact.recorded_at(),
        updated_at: fact.updated_at(),
        valid_at: fact.valid_at(),
        invalid_at: fact.invalid_at(),
        importance: fact.importance(),
        durability: fact.durability(),
        sources: FlowSeq(fact.sources().to_vec()),
        links: FlowSeq(fact.links().to_vec()),
        evidence: FlowSeq(fact.extra_evidence().to_vec()),
    }
}

fn serialize(header: &impl Serialize) -> Result<String, VaultError> {
    let mut options = serde_saphyr::SerializerOptions::default();
    options.prefer_block_scalars = false;
    serde_saphyr::to_string_with_options(header, options)
        .map_err(|error| VaultError::Format(error.to_string()))
}

pub fn parse(text: &str) -> Result<Memory, VaultError> {
    let rest = text
        .strip_prefix(FENCE)
        .ok_or_else(|| VaultError::Format("the file does not start with ---".into()))?;
    let end = rest.find("\n---\n").or_else(|| rest.strip_suffix("\n---").map(str::len));
    let end = end.ok_or_else(|| VaultError::Format("the header is not closed with ---".into()))?;
    let header = &rest[..=end];
    let body = rest.get(end + 5..).unwrap_or_default().trim().to_owned();
    let kind: KindOnly = deserialize(header)?;
    match kind.kind {
        MemoryKind::Fact => fact_from(deserialize(header)?, body).map(Memory::Fact),
        MemoryKind::Entity => entity_from(deserialize(header)?, body).map(Memory::Entity),
    }
}

fn deserialize<T: serde::de::DeserializeOwned>(header: &str) -> Result<T, VaultError> {
    serde_saphyr::from_str(header).map_err(|error| VaultError::Format(error.to_string()))
}

fn fact_from(header: FactHeader, content: String) -> Result<Fact, VaultError> {
    Fact::restore(StoredFact {
        id: header.id,
        content,
        tags: header.tags.0,
        about: header.about.0,
        recorded_at: header.recorded_at,
        updated_at: header.updated_at,
        valid_at: header.valid_at,
        invalid_at: header.invalid_at,
        importance: header.importance,
        durability: header.durability,
        sources: header.sources.0,
        links: header.links.0,
        evidence: header.evidence.0,
    })
    .map_err(|error| VaultError::Format(error.to_string()))
}

fn entity_from(header: EntityHeader, description: String) -> Result<neomem_core::Entity, VaultError> {
    neomem_core::Entity::restore(StoredEntity {
        id: header.id,
        kind: header.entity_kind,
        name: header.name,
        aliases: header.aliases.0,
        description,
        recorded_at: header.recorded_at,
        updated_at: header.updated_at,
        merged_from: header.merged_from.0,
        merged_into: header.merged_into,
        relations: header.relations.0,
        links: header.links.0,
    })
    .map_err(|error| VaultError::Format(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_core::{FactLinkKind, NewEntity, NewFact};

    fn at(millis: i64) -> EpochMs {
        EpochMs::from_millis(millis)
    }

    fn sample_fact() -> Fact {
        let mut fact = Fact::new(NewFact {
            id: FactId::from_parts(at(1_792_400_100_000), 5),
            content: "Edge's ship date is November 1, 2026.".to_owned(),
            tags: vec![Tag::new("code").unwrap(), Tag::new("edge").unwrap()],
            about: vec![EntityId::from_parts(at(1_792_300_050_000), 6)],
            recorded_at: at(1_792_400_100_000),
            valid_at: at(1_792_400_000_000),
            importance: Importance::new(7).unwrap(),
            durability: Durability::Ordinary,
            sources: vec![IntakeId::from_parts(at(1_792_400_000_000), 7)],
            links: vec![FactLink {
                kind: FactLinkKind::Updates,
                to: FactId::from_parts(at(1_792_300_100_000), 8),
            }],
        })
        .unwrap();
        fact.mark_useful(at(1_792_500_000_000));
        fact
    }

    #[test]
    fn a_fact_file_looks_like_the_spec_and_round_trips() {
        let fact = sample_fact();
        let text = render(&Memory::Fact(fact.clone())).unwrap();
        assert!(text.starts_with("---\nid: fct_"));
        assert!(text.contains("tags: [code, edge]\n"));
        assert!(text.contains("invalid_at: null\n"));
        assert!(text.contains("links: [{type: updates, to: fct_"));
        assert!(text.ends_with("---\nEdge's ship date is November 1, 2026.\n"));
        assert_eq!(parse(&text).unwrap(), Memory::Fact(fact));
    }

    #[test]
    fn an_entity_file_round_trips_with_awkward_names() {
        let mut entity = neomem_core::Entity::new(NewEntity {
            id: EntityId::from_parts(at(1), 1),
            kind: Label::new("person").unwrap(),
            name: "Sam \"Sammy\" Kim: CTO".to_owned(),
            aliases: vec!["sam@acme.com".to_owned(), "- dash".to_owned()],
            description: "Works on Edge.\n\nSecond paragraph.".to_owned(),
            recorded_at: at(1),
        })
        .unwrap();
        entity.add_alias("null", at(2));
        let text = render(&Memory::Entity(entity.clone())).unwrap();
        assert_eq!(parse(&text).unwrap(), Memory::Entity(entity));
    }

    #[test]
    fn broken_files_are_reported_not_trusted() {
        assert!(parse("no header at all").is_err());
        assert!(parse("---\nid: fct_x\nkind: fact\n---\nbody\n").is_err());
        let fact_without_entity =
            render(&Memory::Fact(sample_fact())).unwrap().replacen("about: [ent_", "about: [], x: [ent_", 1);
        assert!(parse(&fact_without_entity).is_err());
    }
}
