use std::fs;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Feature {
    pub id: String,
    pub does: String,
    pub reached_via: Vec<String>,
    pub module: String,
    pub depends_on: Vec<String>,
    pub proof: String,
    pub status: Status,
    pub last_proof: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Planned,
    Building,
    Proven,
    Broken,
}

impl Feature {
    pub fn crate_name(&self) -> &str {
        self.module
            .split_whitespace()
            .next()
            .unwrap_or(&self.module)
    }
}

pub fn load(path: &Path) -> anyhow::Result<Vec<Feature>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_saphyr::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}
