use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_saphyr::FlowSeq;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Feature {
    pub id: String,
    pub does: String,
    pub reached_via: FlowSeq<Vec<String>>,
    pub module: String,
    pub depends_on: FlowSeq<Vec<String>>,
    pub proof: String,
    pub status: Status,
    pub last_proof: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Planned,
    Building,
    Proven,
    Broken,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Building => "building",
            Self::Proven => "proven",
            Self::Broken => "broken",
        }
    }
}

impl Feature {
    pub fn crate_name(&self) -> &str {
        self.module.split_whitespace().next().unwrap_or(&self.module)
    }

    pub fn dependencies(&self) -> &[String] {
        &self.depends_on.0
    }

    pub fn surfaces(&self) -> &[String] {
        &self.reached_via.0
    }
}

#[derive(Debug)]
pub struct FeatureGraph {
    pub features: Vec<Feature>,
}

impl FeatureGraph {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let features: Vec<Feature> =
            serde_saphyr::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Self::from_features(features)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut options = serde_saphyr::SerializerOptions::default();
        options.prefer_block_scalars = false;
        let text = serde_saphyr::to_string_with_options(&self.features, options)
            .context("serializing the feature graph")?;
        fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn record_proof(&mut self, id: &str, passed: bool, bundle: String) {
        if let Some(feature) = self.features.iter_mut().find(|feature| feature.id == id) {
            feature.status = match (passed, feature.status) {
                (true, _) => Status::Proven,
                (false, Status::Proven | Status::Broken) => Status::Broken,
                (false, Status::Planned | Status::Building) => Status::Building,
            };
            feature.last_proof = Some(bundle);
        }
    }

    pub fn from_features(features: Vec<Feature>) -> anyhow::Result<Self> {
        let graph = Self { features };
        graph.validate()?;
        Ok(graph)
    }

    pub fn feature(&self, id: &str) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.id == id)
    }

    pub fn is_unblocked(&self, feature: &Feature) -> bool {
        feature
            .dependencies()
            .iter()
            .all(|dependency| self.feature(dependency).is_some_and(|found| found.status == Status::Proven))
    }

    pub fn next_to_build(&self) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.status != Status::Proven && self.is_unblocked(feature))
    }

    fn validate(&self) -> anyhow::Result<()> {
        let mut ids = BTreeSet::new();
        for feature in &self.features {
            if !ids.insert(feature.id.as_str()) {
                bail!("feature {} appears twice", feature.id);
            }
        }
        for feature in &self.features {
            if let Some(missing) =
                feature.dependencies().iter().find(|dependency| !ids.contains(dependency.as_str()))
            {
                bail!("feature {} depends on unknown feature {missing}", feature.id);
            }
        }
        self.reject_cycles()
    }

    fn reject_cycles(&self) -> anyhow::Result<()> {
        let mut depth_first =
            CycleSearch { graph: self, finished: BTreeSet::new(), on_path: BTreeSet::new() };
        for feature in &self.features {
            depth_first.visit(&feature.id)?;
        }
        Ok(())
    }
}

struct CycleSearch<'graph> {
    graph: &'graph FeatureGraph,
    finished: BTreeSet<&'graph str>,
    on_path: BTreeSet<&'graph str>,
}

impl<'graph> CycleSearch<'graph> {
    fn visit(&mut self, id: &'graph str) -> anyhow::Result<()> {
        if self.finished.contains(id) {
            return Ok(());
        }
        if !self.on_path.insert(id) {
            bail!("the feature graph has a dependency cycle through {id}");
        }
        if let Some(feature) = self.graph.feature(id) {
            for dependency in feature.dependencies() {
                self.visit(dependency)?;
            }
        }
        self.on_path.remove(id);
        self.finished.insert(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(id: &str, depends_on: &[&str], status: Status) -> Feature {
        Feature {
            id: id.to_owned(),
            does: String::new(),
            reached_via: FlowSeq(Vec::new()),
            module: "neomem-core".to_owned(),
            depends_on: FlowSeq(depends_on.iter().map(ToString::to_string).collect()),
            proof: id.to_owned(),
            status,
            last_proof: None,
            blocked: None,
        }
    }

    #[test]
    fn next_skips_features_whose_dependencies_are_not_proven() {
        let graph = FeatureGraph::from_features(vec![
            feature("daemon", &[], Status::Proven),
            feature("search", &["index"], Status::Planned),
            feature("index", &["daemon"], Status::Planned),
        ])
        .unwrap();
        assert_eq!(graph.next_to_build().map(|found| found.id.as_str()), Some("index"));
    }

    #[test]
    fn cycles_are_rejected() {
        let outcome = FeatureGraph::from_features(vec![
            feature("a", &["b"], Status::Planned),
            feature("b", &["a"], Status::Planned),
        ]);
        assert!(outcome.is_err());
    }

    #[test]
    fn unknown_dependencies_are_rejected() {
        let outcome = FeatureGraph::from_features(vec![feature("a", &["ghost"], Status::Planned)]);
        assert!(outcome.is_err());
    }
}
