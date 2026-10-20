use serde::{Deserialize, Serialize};

use crate::LinkKind;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub k: usize,
    pub contradicts_weight: f32,
    pub extends_weight: f32,
    pub part_of_weight: f32,
    pub keep_ratio: f32,
    pub linked_per_seed: usize,
    pub near_duplicate: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            k: 10,
            contradicts_weight: 1.0,
            extends_weight: 0.9,
            part_of_weight: 0.8,
            keep_ratio: 0.8,
            linked_per_seed: 2,
            near_duplicate: 0.92,
        }
    }
}

impl SearchConfig {
    pub fn weight_of(&self, kind: &LinkKind) -> Option<f32> {
        match kind {
            LinkKind::Contradicts => Some(self.contradicts_weight),
            LinkKind::Extends => Some(self.extends_weight),
            LinkKind::PartOf => Some(self.part_of_weight),
            _ => None,
        }
    }
}
