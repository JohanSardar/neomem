use anyhow::bail;

use crate::graph::{self, Feature};
use crate::{changes, repo};

#[derive(Debug)]
pub enum Selection {
    One(String),
    Changed,
    All,
}

pub fn run(selection: &Selection) -> anyhow::Result<bool> {
    let root = repo::root()?;
    let features = graph::load(&repo::feature_graph_path(&root))?;
    let chosen = choose(&features, selection, &root)?;
    if chosen.is_empty() {
        eprintln!("prove: no features selected");
        return Ok(true);
    }
    bail!(
        "no proof scenarios exist yet for {} feature(s)",
        chosen.len()
    )
}

fn choose<'graph>(
    features: &'graph [Feature],
    selection: &Selection,
    root: &std::path::Path,
) -> anyhow::Result<Vec<&'graph Feature>> {
    match selection {
        Selection::All => Ok(features.iter().collect()),
        Selection::One(id) => match features.iter().find(|feature| &feature.id == id) {
            Some(feature) => Ok(vec![feature]),
            None => bail!("feature {id} is not in the feature graph"),
        },
        Selection::Changed => {
            let changed = changes::changed_files(root)?;
            Ok(features
                .iter()
                .filter(|feature| {
                    let crate_dir = format!("crates/{}/", feature.crate_name());
                    changed.iter().any(|file| file.starts_with(&crate_dir))
                })
                .collect())
        }
    }
}
