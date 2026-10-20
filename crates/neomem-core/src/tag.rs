use std::fmt;

use serde::{Deserialize, Serialize};

pub const STARTING_TAGS: &[&str] = &[
    "conversation",
    "agent-session",
    "work-session",
    "code",
    "health",
    "sleep",
    "money",
    "purchase",
    "calendar",
    "social",
    "school",
    "travel",
    "place",
    "media",
    "raw",
    "system",
];

const HIDDEN_BY_DEFAULT: &[&str] = &["raw", "system"];
const LONGEST_TAG: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Tag(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a tag: use 1-48 lowercase letters, digits and hyphens")]
pub struct InvalidTag(pub String);

impl Tag {
    pub fn new(text: &str) -> Result<Self, InvalidTag> {
        let normalized = text.trim().to_lowercase().replace([' ', '_'], "-");
        let well_formed = !normalized.is_empty()
            && normalized.len() <= LONGEST_TAG
            && normalized.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
            && !normalized.starts_with('-')
            && !normalized.ends_with('-');
        if well_formed { Ok(Self(normalized)) } else { Err(InvalidTag(text.to_owned())) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_hidden_by_default(&self) -> bool {
        HIDDEN_BY_DEFAULT.contains(&self.0.as_str())
    }
}

impl TryFrom<String> for Tag {
    type Error = InvalidTag;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<Tag> for String {
    fn from(tag: Tag) -> Self {
        tag.0
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_normalized_to_lowercase_kebab_case() {
        assert_eq!(Tag::new(" Work Session ").unwrap().as_str(), "work-session");
        assert!(Tag::new("").is_err());
        assert!(Tag::new("health!").is_err());
        assert!(Tag::new("-raw").is_err());
    }

    #[test]
    fn raw_and_system_are_hidden_by_default() {
        assert!(Tag::new("raw").unwrap().is_hidden_by_default());
        assert!(!Tag::new("health").unwrap().is_hidden_by_default());
    }

    #[test]
    fn every_starting_tag_is_valid() {
        assert!(STARTING_TAGS.iter().all(|tag| Tag::new(tag).is_ok()));
    }
}
