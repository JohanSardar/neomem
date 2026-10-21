use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{EpochMs, MemoryId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExternalRef(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not an external reference: expected ext:<system>/<id>")]
pub struct InvalidExternalRef(pub String);

impl ExternalRef {
    pub fn system(&self) -> &str {
        self.0.strip_prefix("ext:").and_then(|text| text.split_once('/')).map_or("", |(system, _)| system)
    }
}

impl FromStr for ExternalRef {
    type Err = InvalidExternalRef;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidExternalRef(text.to_owned());
        let (system, id) =
            text.strip_prefix("ext:").and_then(|body| body.split_once('/')).ok_or_else(invalid)?;
        let valid_system = system.len() <= 48
            && system.starts_with(|letter: char| letter.is_ascii_lowercase())
            && system.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        let valid_id = !id.is_empty()
            && id.len() <= 256
            && id != "."
            && id != ".."
            && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte));
        if valid_system && valid_id { Ok(Self(text.to_owned())) } else { Err(invalid()) }
    }
}

impl TryFrom<String> for ExternalRef {
    type Error = InvalidExternalRef;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<ExternalRef> for String {
    fn from(reference: ExternalRef) -> Self {
        reference.0
    }
}

impl fmt::Display for ExternalRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalLinkKind {
    Mentions,
}

impl ExternalLinkKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mentions => "mentions",
        }
    }
}

impl FromStr for ExternalLinkKind {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "mentions" => Ok(Self::Mentions),
            _ => Err(format!("unknown external link type `{text}`: expected mentions")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalLink {
    pub external: ExternalRef,
    pub to: MemoryId,
    pub kind: ExternalLinkKind,
    pub recorded_at: EpochMs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_references_validate_both_components() {
        assert_eq!("ext:episodes/epi_123".parse::<ExternalRef>().unwrap().system(), "episodes");
        for invalid in [
            "episodes/epi_123",
            "ext:/id",
            "ext:UPPER/id",
            "ext:a/",
            "ext:a/..",
            "ext:a/a/b",
            "ext:a/has space",
            "ext:a/💭",
        ] {
            assert!(invalid.parse::<ExternalRef>().is_err(), "{invalid}");
        }
        assert!(format!("ext:a/{}", "x".repeat(257)).parse::<ExternalRef>().is_err());
        assert!("unknown".parse::<ExternalLinkKind>().is_err());
    }
}
