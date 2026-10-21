use serde::{Deserialize, Serialize};

use crate::{EntityId, EpochMs, FactId, IntakeId, Tag};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactLinkKind {
    Updates,
    Extends,
    Contradicts,
    PartOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FactLink {
    #[serde(rename = "type")]
    pub kind: FactLinkKind,
    pub to: FactId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Durability {
    Stable,
    Preference,
    Ordinary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Importance(u8);

impl Importance {
    pub fn new(value: u8) -> Result<Self, InvalidFact> {
        if (1..=10).contains(&value) { Ok(Self(value)) } else { Err(InvalidFact::Importance(value)) }
    }

    pub fn get(self) -> u8 {
        self.0
    }

    pub fn normalized(self) -> f32 {
        f32::from(self.0 - 1) / 9.0
    }
}

impl TryFrom<u8> for Importance {
    type Error = InvalidFact;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Importance> for u8 {
    fn from(importance: Importance) -> Self {
        importance.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidFact {
    #[error("importance must be 1 to 10, not {0}")]
    Importance(u8),
    #[error("a fact needs content")]
    EmptyContent,
    #[error("every fact is about at least one entity")]
    AboutNothing,
    #[error("every fact cites at least one intake record")]
    NoSources,
    #[error("a fact cannot link to itself")]
    SelfLink,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewFact {
    pub id: FactId,
    pub content: String,
    pub tags: Vec<Tag>,
    pub about: Vec<EntityId>,
    pub recorded_at: EpochMs,
    pub valid_at: EpochMs,
    pub importance: Importance,
    pub durability: Durability,
    pub sources: Vec<IntakeId>,
    pub links: Vec<FactLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredFact")]
pub struct Fact {
    id: FactId,
    content: String,
    tags: Vec<Tag>,
    about: Vec<EntityId>,
    recorded_at: EpochMs,
    updated_at: EpochMs,
    valid_at: EpochMs,
    invalid_at: Option<EpochMs>,
    importance: Importance,
    durability: Durability,
    sources: Vec<IntakeId>,
    links: Vec<FactLink>,
    evidence: Vec<EpochMs>,
}

impl Fact {
    pub fn new(new: NewFact) -> Result<Self, InvalidFact> {
        let fact = Self {
            id: new.id,
            content: new.content.trim().to_owned(),
            tags: new.tags,
            about: new.about,
            recorded_at: new.recorded_at,
            updated_at: new.recorded_at,
            valid_at: new.valid_at,
            invalid_at: None,
            importance: new.importance,
            durability: new.durability,
            sources: new.sources,
            links: new.links,
            evidence: Vec::new(),
        };
        fact.validate()?;
        Ok(fact)
    }

    pub fn validate(&self) -> Result<(), InvalidFact> {
        if self.content.trim().is_empty() {
            return Err(InvalidFact::EmptyContent);
        }
        if self.about.is_empty() {
            return Err(InvalidFact::AboutNothing);
        }
        if self.sources.is_empty() {
            return Err(InvalidFact::NoSources);
        }
        if self.links.iter().any(|link| link.to == self.id) {
            return Err(InvalidFact::SelfLink);
        }
        Ok(())
    }

    pub fn id(&self) -> FactId {
        self.id
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    pub fn about(&self) -> &[EntityId] {
        &self.about
    }

    pub fn recorded_at(&self) -> EpochMs {
        self.recorded_at
    }

    pub fn updated_at(&self) -> EpochMs {
        self.updated_at
    }

    pub fn valid_at(&self) -> EpochMs {
        self.valid_at
    }

    pub fn invalid_at(&self) -> Option<EpochMs> {
        self.invalid_at
    }

    pub fn is_valid_at(&self, moment: EpochMs) -> bool {
        self.valid_at <= moment && self.invalid_at.is_none_or(|ended| moment < ended)
    }

    pub fn importance(&self) -> Importance {
        self.importance
    }

    pub fn durability(&self) -> Durability {
        self.durability
    }

    pub fn sources(&self) -> &[IntakeId] {
        &self.sources
    }

    pub fn links(&self) -> &[FactLink] {
        &self.links
    }

    pub fn evidence(&self) -> impl Iterator<Item = EpochMs> + '_ {
        std::iter::once(self.recorded_at).chain(self.evidence.iter().copied())
    }

    pub fn extra_evidence(&self) -> &[EpochMs] {
        &self.evidence
    }

    pub fn invalidate(&mut self, at: EpochMs, now: EpochMs) -> bool {
        if self.invalid_at.is_some() {
            return false;
        }
        self.invalid_at = Some(at);
        self.updated_at = now;
        true
    }

    pub fn link(&mut self, link: FactLink, now: EpochMs) -> Result<bool, InvalidFact> {
        if link.to == self.id {
            return Err(InvalidFact::SelfLink);
        }
        if self.links.contains(&link) {
            return Ok(false);
        }
        self.links.push(link);
        self.updated_at = now;
        Ok(true)
    }

    pub fn link_entity(&mut self, entity: EntityId, now: EpochMs) -> bool {
        if self.about.contains(&entity) {
            return false;
        }
        self.about.push(entity);
        self.updated_at = now;
        true
    }

    pub fn attach_source(&mut self, source: IntakeId, now: EpochMs) -> bool {
        if self.sources.contains(&source) {
            return false;
        }
        self.sources.push(source);
        self.evidence.push(now);
        self.updated_at = now;
        true
    }

    pub fn mark_useful(&mut self, now: EpochMs) {
        self.evidence.push(now);
        self.updated_at = now;
    }

    pub fn restore(stored: StoredFact) -> Result<Self, InvalidFact> {
        let fact = Self {
            id: stored.id,
            content: stored.content,
            tags: stored.tags,
            about: stored.about,
            recorded_at: stored.recorded_at,
            updated_at: stored.updated_at,
            valid_at: stored.valid_at,
            invalid_at: stored.invalid_at,
            importance: stored.importance,
            durability: stored.durability,
            sources: stored.sources,
            links: stored.links,
            evidence: stored.evidence,
        };
        fact.validate()?;
        Ok(fact)
    }
}

impl TryFrom<StoredFact> for Fact {
    type Error = InvalidFact;

    fn try_from(stored: StoredFact) -> Result<Self, Self::Error> {
        Self::restore(stored)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StoredFact {
    pub id: FactId,
    pub content: String,
    pub tags: Vec<Tag>,
    pub about: Vec<EntityId>,
    pub recorded_at: EpochMs,
    pub updated_at: EpochMs,
    pub valid_at: EpochMs,
    pub invalid_at: Option<EpochMs>,
    pub importance: Importance,
    pub durability: Durability,
    pub sources: Vec<IntakeId>,
    #[serde(default)]
    pub links: Vec<FactLink>,
    #[serde(default)]
    pub evidence: Vec<EpochMs>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: i64) -> EpochMs {
        EpochMs::from_millis(millis)
    }

    fn ship_date() -> Fact {
        Fact::new(NewFact {
            id: FactId::from_parts(at(100), 1),
            content: "Edge ships on November 1, 2026.".to_owned(),
            tags: vec![Tag::new("code").unwrap()],
            about: vec![EntityId::from_parts(at(50), 1)],
            recorded_at: at(100),
            valid_at: at(90),
            importance: Importance::new(7).unwrap(),
            durability: Durability::Ordinary,
            sources: vec![IntakeId::from_parts(at(99), 1)],
            links: vec![],
        })
        .unwrap()
    }

    #[test]
    fn a_fact_needs_an_entity_and_a_source() {
        let mut new = NewFact {
            id: FactId::from_parts(at(1), 1),
            content: "x".to_owned(),
            tags: vec![],
            about: vec![],
            recorded_at: at(1),
            valid_at: at(1),
            importance: Importance::new(5).unwrap(),
            durability: Durability::Ordinary,
            sources: vec![IntakeId::from_parts(at(1), 2)],
            links: vec![],
        };
        assert_eq!(Fact::new(new.clone()), Err(InvalidFact::AboutNothing));
        new.about = vec![EntityId::from_parts(at(1), 3)];
        new.sources.clear();
        assert_eq!(Fact::new(new), Err(InvalidFact::NoSources));
    }

    #[test]
    fn invalidation_happens_once_and_keeps_the_fact() {
        let mut fact = ship_date();
        assert!(fact.invalidate(at(500), at(600)));
        assert!(!fact.invalidate(at(700), at(800)));
        assert_eq!(fact.invalid_at(), Some(at(500)));
        assert!(fact.is_valid_at(at(400)));
        assert!(!fact.is_valid_at(at(500)));
    }

    #[test]
    fn a_repeated_source_is_evidence_only_once() {
        let mut fact = ship_date();
        let again = IntakeId::from_parts(at(300), 9);
        assert!(fact.attach_source(again, at(300)));
        assert!(!fact.attach_source(again, at(400)));
        assert_eq!(fact.evidence().count(), 2);
    }

    #[test]
    fn deserializing_a_fact_checks_it_like_construction_does() {
        let mut json = serde_json::to_value(ship_date()).unwrap();
        assert!(serde_json::from_value::<Fact>(json.clone()).is_ok());
        json["about"] = serde_json::json!([]);
        assert!(serde_json::from_value::<Fact>(json).is_err());
    }

    #[test]
    fn importance_stays_between_one_and_ten() {
        assert!(Importance::new(0).is_err());
        assert!(Importance::new(11).is_err());
        assert!((Importance::new(10).unwrap().normalized() - 1.0).abs() < f32::EPSILON);
    }
}
