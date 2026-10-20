use crate::{Entity, Fact, MemoryId};

#[derive(Debug, Clone, PartialEq)]
pub enum Memory {
    Fact(Fact),
    Entity(Entity),
}

impl Memory {
    pub fn id(&self) -> MemoryId {
        match self {
            Self::Fact(fact) => MemoryId::Fact(fact.id()),
            Self::Entity(entity) => MemoryId::Entity(entity.id()),
        }
    }

    pub fn embedding_text(&self) -> String {
        match self {
            Self::Fact(fact) => fact.content().to_owned(),
            Self::Entity(entity) => {
                let aliases = entity.aliases().join(", ");
                let also = if aliases.is_empty() { String::new() } else { format!(" (also {aliases})") };
                let description = if entity.description().is_empty() {
                    String::new()
                } else {
                    format!(": {}", entity.description())
                };
                format!("{}{also}, a {}{description}", entity.name(), entity.kind())
            }
        }
    }
}
