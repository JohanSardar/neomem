use std::path::{Path, PathBuf};
use std::process::Command;

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

pub fn shared_models_dir(root: &Path) -> PathBuf {
    let common_git_dir = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
    let main_checkout = common_git_dir.as_deref().and_then(Path::parent).unwrap_or(root);
    main_checkout.join("models")
}
