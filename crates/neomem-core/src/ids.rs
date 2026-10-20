use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::EpochMs;
use crate::ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` is not a valid {expected} ID")]
pub struct InvalidId {
    pub text: String,
    pub expected: &'static str,
}

macro_rules! prefixed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Ulid);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn from_parts(timestamp: EpochMs, randomness: u128) -> Self {
                Self(Ulid::from_parts(timestamp, randomness))
            }

            pub fn timestamp(self) -> EpochMs {
                self.0.timestamp()
            }

            pub const fn to_u128(self) -> u128 {
                self.0.to_u128()
            }

            pub const fn from_u128(value: u128) -> Self {
                Self(Ulid::from_u128(value))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}_{}", $prefix, self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let invalid = || InvalidId { text: text.to_owned(), expected: $prefix };
                let body = text.strip_prefix(concat!($prefix, "_")).ok_or_else(invalid)?;
                body.parse::<Ulid>().map(Self).map_err(|_| invalid())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

prefixed_id!(FactId, "fct");
prefixed_id!(EntityId, "ent");
prefixed_id!(IntakeId, "int");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MemoryId {
    Fact(FactId),
    Entity(EntityId),
}

impl fmt::Display for MemoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fact(id) => id.fmt(formatter),
            Self::Entity(id) => id.fmt(formatter),
        }
    }
}

impl FromStr for MemoryId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.starts_with(FactId::PREFIX) {
            return text.parse().map(Self::Fact);
        }
        text.parse()
            .map(Self::Entity)
            .map_err(|_| InvalidId { text: text.to_owned(), expected: "fct or ent" })
    }
}

impl From<FactId> for MemoryId {
    fn from(id: FactId) -> Self {
        Self::Fact(id)
    }
}

impl From<EntityId> for MemoryId {
    fn from(id: EntityId) -> Self {
        Self::Entity(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_carry_their_prefix_and_refuse_the_others() {
        let fact = FactId::from_parts(EpochMs::from_millis(1_760_796_300_000), 7);
        let text = fact.to_string();
        assert!(text.starts_with("fct_"));
        assert_eq!(text.parse::<FactId>().unwrap(), fact);
        assert!(text.parse::<EntityId>().is_err());
        assert!(text.replace("fct_", "int_").parse::<FactId>().is_err());
    }

    #[test]
    fn memory_ids_dispatch_on_the_prefix() {
        let entity = EntityId::from_parts(EpochMs::from_millis(5), 9);
        assert_eq!(entity.to_string().parse::<MemoryId>().unwrap(), MemoryId::Entity(entity));
        let json = serde_json::to_string(&MemoryId::Entity(entity)).unwrap();
        assert_eq!(serde_json::from_str::<MemoryId>(&json).unwrap(), MemoryId::Entity(entity));
    }
}
