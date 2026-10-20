use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{EpochMs, IntakeId, Tag};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Source(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidIntake {
    #[error("`{0}` is not a source name: use 1-64 lowercase letters, digits, dots, hyphens or underscores")]
    Source(String),
    #[error("the occurred span ends before it starts")]
    BackwardsSpan,
    #[error("an intake record needs content")]
    EmptyContent,
}

impl Source {
    pub fn new(text: &str) -> Result<Self, InvalidIntake> {
        let well_formed = !text.is_empty()
            && text.len() <= 64
            && text.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || "-_.".contains(character)
            });
        if well_formed { Ok(Self(text.to_owned())) } else { Err(InvalidIntake::Source(text.to_owned())) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Source {
    type Error = InvalidIntake;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<Source> for String {
    fn from(source: Source) -> Self {
        source.0
    }
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: EpochMs,
    pub end: EpochMs,
}

impl Span {
    pub fn new(start: EpochMs, end: EpochMs) -> Result<Self, InvalidIntake> {
        if end < start { Err(InvalidIntake::BackwardsSpan) } else { Ok(Self { start, end }) }
    }

    pub fn instant(at: EpochMs) -> Self {
        Self { start: at, end: at }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeRecord {
    pub source: Source,
    pub occurred: Span,
    #[serde(default)]
    pub tags: Vec<Tag>,
    pub content: String,
    #[serde(default)]
    pub entity_hints: Vec<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
}

impl IntakeRecord {
    pub fn validate(&self) -> Result<(), InvalidIntake> {
        if self.content.trim().is_empty() {
            return Err(InvalidIntake::EmptyContent);
        }
        Span::new(self.occurred.start, self.occurred.end).map(drop)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedIntake {
    pub id: IntakeId,
    pub recorded_at: EpochMs,
    pub record: IntakeRecord,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_plain_lowercase_names() {
        assert!(Source::new("oura-digest").is_ok());
        assert!(Source::new("claude-code").is_ok());
        assert!(Source::new("Oura Digest").is_err());
    }

    #[test]
    fn spans_never_run_backwards() {
        let later = EpochMs::from_millis(10);
        assert_eq!(Span::new(later, EpochMs::from_millis(5)), Err(InvalidIntake::BackwardsSpan));
    }

    #[test]
    fn a_record_needs_content() {
        let record = IntakeRecord {
            source: Source::new("note").unwrap(),
            occurred: Span::instant(EpochMs::from_millis(1)),
            tags: vec![],
            content: "   ".to_owned(),
            entity_hints: vec![],
            source_ref: None,
            session: None,
        };
        assert_eq!(record.validate(), Err(InvalidIntake::EmptyContent));
    }
}
