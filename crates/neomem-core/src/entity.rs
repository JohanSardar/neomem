use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{EntityId, EpochMs};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Label(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidEntity {
    #[error("`{0}` is not a label: use 1-48 lowercase letters, digits, hyphens or underscores")]
    Label(String),
    #[error("an entity needs a name")]
    EmptyName,
    #[error("an entity cannot link or relate to itself")]
    SelfLink,
}

impl Label {
    pub fn new(text: &str) -> Result<Self, InvalidEntity> {
        let normalized = text.trim().to_lowercase().replace(' ', "_");
        let well_formed = !normalized.is_empty()
            && normalized.len() <= 48
            && normalized.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || "-_".contains(character)
            });
        if well_formed { Ok(Self(normalized)) } else { Err(InvalidEntity::Label(text.to_owned())) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn thing() -> Self {
        Self("thing".to_owned())
    }
}

impl TryFrom<String> for Label {
    type Error = InvalidEntity;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<Label> for String {
    fn from(label: Label) -> Self {
        label.0
    }
}

impl fmt::Display for Label {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    pub label: Label,
    pub to: EntityId,
    pub valid_at: EpochMs,
    pub invalid_at: Option<EpochMs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityLinkKind {
    PartOf,
    PossiblySame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityLink {
    #[serde(rename = "type")]
    pub kind: EntityLinkKind,
    pub to: EntityId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntity {
    pub id: EntityId,
    pub kind: Label,
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub recorded_at: EpochMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredEntity")]
pub struct Entity {
    id: EntityId,
    kind: Label,
    name: String,
    aliases: Vec<String>,
    description: String,
    recorded_at: EpochMs,
    updated_at: EpochMs,
    merged_from: Vec<EntityId>,
    merged_into: Option<EntityId>,
    relations: Vec<Relation>,
    links: Vec<EntityLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredEntity {
    pub id: EntityId,
    pub kind: Label,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: String,
    pub recorded_at: EpochMs,
    pub updated_at: EpochMs,
    #[serde(default)]
    pub merged_from: Vec<EntityId>,
    #[serde(default)]
    pub merged_into: Option<EntityId>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub links: Vec<EntityLink>,
}

impl Entity {
    pub fn new(new: NewEntity) -> Result<Self, InvalidEntity> {
        Self::restore(StoredEntity {
            id: new.id,
            kind: new.kind,
            name: new.name.trim().to_owned(),
            aliases: new.aliases,
            description: new.description,
            recorded_at: new.recorded_at,
            updated_at: new.recorded_at,
            merged_from: Vec::new(),
            merged_into: None,
            relations: Vec::new(),
            links: Vec::new(),
        })
    }

    pub fn restore(stored: StoredEntity) -> Result<Self, InvalidEntity> {
        if stored.name.trim().is_empty() {
            return Err(InvalidEntity::EmptyName);
        }
        let points_at_itself = stored.relations.iter().any(|relation| relation.to == stored.id)
            || stored.links.iter().any(|link| link.to == stored.id)
            || stored.merged_into == Some(stored.id);
        if points_at_itself {
            return Err(InvalidEntity::SelfLink);
        }
        Ok(Self {
            id: stored.id,
            kind: stored.kind,
            name: stored.name,
            aliases: stored.aliases,
            description: stored.description,
            recorded_at: stored.recorded_at,
            updated_at: stored.updated_at,
            merged_from: stored.merged_from,
            merged_into: stored.merged_into,
            relations: stored.relations,
            links: stored.links,
        })
    }

    pub fn id(&self) -> EntityId {
        self.id
    }

    pub fn kind(&self) -> &Label {
        &self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn recorded_at(&self) -> EpochMs {
        self.recorded_at
    }

    pub fn updated_at(&self) -> EpochMs {
        self.updated_at
    }

    pub fn merged_from(&self) -> &[EntityId] {
        &self.merged_from
    }

    pub fn merged_into(&self) -> Option<EntityId> {
        self.merged_into
    }

    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    pub fn links(&self) -> &[EntityLink] {
        &self.links
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.aliases.iter().map(String::as_str))
    }

    pub fn add_alias(&mut self, alias: &str, now: EpochMs) -> bool {
        let alias = alias.trim();
        if alias.is_empty() || self.names().any(|known| known.eq_ignore_ascii_case(alias)) {
            return false;
        }
        self.aliases.push(alias.to_owned());
        self.updated_at = now;
        true
    }

    pub fn relate(&mut self, relation: Relation, now: EpochMs) -> Result<bool, InvalidEntity> {
        if relation.to == self.id {
            return Err(InvalidEntity::SelfLink);
        }
        if self.relations.contains(&relation) {
            return Ok(false);
        }
        self.relations.push(relation);
        self.updated_at = now;
        Ok(true)
    }

    pub fn link(&mut self, link: EntityLink, now: EpochMs) -> Result<bool, InvalidEntity> {
        if link.to == self.id {
            return Err(InvalidEntity::SelfLink);
        }
        if self.links.contains(&link) {
            return Ok(false);
        }
        self.links.push(link);
        self.updated_at = now;
        Ok(true)
    }

    pub fn absorb(&mut self, duplicate: &Self, now: EpochMs) -> Result<(), InvalidEntity> {
        if duplicate.id == self.id {
            return Err(InvalidEntity::SelfLink);
        }
        for name in duplicate.names() {
            self.add_alias(name, now);
        }
        if !self.merged_from.contains(&duplicate.id) {
            self.merged_from.push(duplicate.id);
        }
        self.updated_at = now;
        Ok(())
    }

    pub fn retire_into(&mut self, survivor: EntityId, now: EpochMs) -> Result<(), InvalidEntity> {
        if survivor == self.id {
            return Err(InvalidEntity::SelfLink);
        }
        self.merged_into = Some(survivor);
        self.updated_at = now;
        Ok(())
    }
}

impl TryFrom<StoredEntity> for Entity {
    type Error = InvalidEntity;

    fn try_from(stored: StoredEntity) -> Result<Self, Self::Error> {
        Self::restore(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(name: &str, seed: u128) -> Entity {
        Entity::new(NewEntity {
            id: EntityId::from_parts(EpochMs::from_millis(1), seed),
            kind: Label::new("person").unwrap(),
            name: name.to_owned(),
            aliases: vec![],
            description: String::new(),
            recorded_at: EpochMs::from_millis(1),
        })
        .unwrap()
    }

    #[test]
    fn merging_keeps_every_name_and_records_where_it_came_from() {
        let mut survivor = entity("Sam Kim", 1);
        let mut duplicate = entity("Samuel Kim", 2);
        duplicate.add_alias("sam@acme.com", EpochMs::from_millis(2));
        survivor.absorb(&duplicate, EpochMs::from_millis(3)).unwrap();
        duplicate.retire_into(survivor.id(), EpochMs::from_millis(3)).unwrap();
        assert_eq!(survivor.aliases(), ["Samuel Kim", "sam@acme.com"]);
        assert_eq!(survivor.merged_from(), [duplicate.id()]);
        assert_eq!(duplicate.merged_into(), Some(survivor.id()));
    }

    #[test]
    fn aliases_are_not_repeated_case_insensitively() {
        let mut sam = entity("Sam Kim", 1);
        assert!(!sam.add_alias("sam kim", EpochMs::from_millis(2)));
    }

    #[test]
    fn relation_labels_are_open_but_well_formed() {
        assert_eq!(Label::new("Works At").unwrap().as_str(), "works_at");
        assert!(Label::new("sister of!").is_err());
    }
}
