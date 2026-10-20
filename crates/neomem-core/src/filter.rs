use serde::{Deserialize, Serialize};

use crate::{EntityId, EpochMs, Tag};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub entities: Vec<EntityId>,
    #[serde(default)]
    pub from: Option<EpochMs>,
    #[serde(default)]
    pub to: Option<EpochMs>,
    #[serde(default)]
    pub include_hidden_tags: bool,
}

impl Filter {
    pub fn accepts_tags(&self, tags: &[Tag]) -> bool {
        let shown = self.include_hidden_tags || !tags.iter().any(Tag::is_hidden_by_default);
        let wanted = self.tags.is_empty() || tags.iter().any(|tag| self.tags.contains(tag));
        shown && wanted
    }

    pub fn accepts_entities(&self, about: &[EntityId]) -> bool {
        self.entities.is_empty() || about.iter().any(|entity| self.entities.contains(entity))
    }

    pub fn accepts_validity(&self, valid_at: EpochMs, invalid_at: Option<EpochMs>) -> bool {
        let starts_before_window_ends = self.to.is_none_or(|to| valid_at <= to);
        let ends_after_window_starts =
            self.from.is_none_or(|from| invalid_at.is_none_or(|ended| ended > from));
        starts_before_window_ends && ends_after_window_starts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str) -> Tag {
        Tag::new(name).unwrap()
    }

    #[test]
    fn raw_and_system_stay_hidden_unless_asked_for() {
        let default = Filter::default();
        assert!(!default.accepts_tags(&[tag("raw")]));
        assert!(default.accepts_tags(&[tag("health")]));
        let asking = Filter { include_hidden_tags: true, ..Filter::default() };
        assert!(asking.accepts_tags(&[tag("system")]));
    }

    #[test]
    fn a_time_window_keeps_facts_valid_at_some_point_inside_it() {
        let window = Filter {
            from: Some(EpochMs::from_millis(100)),
            to: Some(EpochMs::from_millis(200)),
            ..Filter::default()
        };
        assert!(window.accepts_validity(EpochMs::from_millis(50), None));
        assert!(window.accepts_validity(EpochMs::from_millis(150), Some(EpochMs::from_millis(300))));
        assert!(!window.accepts_validity(EpochMs::from_millis(250), None));
        assert!(!window.accepts_validity(EpochMs::from_millis(10), Some(EpochMs::from_millis(100))));
    }
}
