use std::path::{Path, PathBuf};

use anyhow::Context;

pub fn root() -> anyhow::Result<PathBuf> {
    let control_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    control_dir
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .context("neomem-control must live in crates/ below the workspace root")
}

pub fn feature_graph_path(root: &Path) -> PathBuf {
    root.join(".agents/skills/neomem-control/feature-graph.yaml")
}
