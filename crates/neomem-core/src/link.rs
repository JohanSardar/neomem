use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{EntityLinkKind, FactLinkKind, Label};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Updates,
    Extends,
    Contradicts,
    PartOf,
    DerivedFrom,
    About,
    PossiblySame,
    Corrects,
    Relation(Label),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Outgoing,
    Incoming,
}

impl From<FactLinkKind> for LinkKind {
    fn from(kind: FactLinkKind) -> Self {
        match kind {
            FactLinkKind::Updates => Self::Updates,
            FactLinkKind::Extends => Self::Extends,
            FactLinkKind::Contradicts => Self::Contradicts,
            FactLinkKind::PartOf => Self::PartOf,
        }
    }
}

impl From<EntityLinkKind> for LinkKind {
    fn from(kind: EntityLinkKind) -> Self {
        match kind {
            EntityLinkKind::PartOf => Self::PartOf,
            EntityLinkKind::PossiblySame => Self::PossiblySame,
        }
    }
}

impl LinkKind {
    pub fn name(&self) -> &str {
        match self {
            Self::Updates => "updates",
            Self::Extends => "extends",
            Self::Contradicts => "contradicts",
            Self::PartOf => "part_of",
            Self::DerivedFrom => "derived_from",
            Self::About => "about",
            Self::PossiblySame => "possibly_same",
            Self::Corrects => "corrects",
            Self::Relation(label) => label.as_str(),
        }
    }
}

impl fmt::Display for LinkKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
