use std::path::{Path, PathBuf};

use anyhow::Context;

const GRAPH_FROM_ROOT: &str = ".agents/skills/neomem-control/feature-graph.yaml";

pub fn root() -> anyhow::Result<PathBuf> {
    let start = std::env::current_dir().context("reading the current directory")?;
    start
        .ancestors()
        .find(|candidate| candidate.join(GRAPH_FROM_ROOT).is_file())
        .map(Path::to_path_buf)
        .context("run neomemctl inside the neomem repository")
}

pub fn feature_graph_path(root: &Path) -> PathBuf {
    root.join(GRAPH_FROM_ROOT)
}
