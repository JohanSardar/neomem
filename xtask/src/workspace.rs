use std::path::{Path, PathBuf};

use anyhow::Context;
use walkdir::WalkDir;

pub fn root() -> anyhow::Result<PathBuf> {
    let xtask_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask_dir
        .parent()
        .map(Path::to_path_buf)
        .context("xtask must live one level below the workspace root")
}

pub fn rust_sources(root: &Path) -> Vec<PathBuf> {
    ["crates", "xtask"]
        .iter()
        .map(|top| root.join(top))
        .flat_map(|top| WalkDir::new(top).into_iter().filter_map(Result::ok))
        .filter(|entry| !entry.path().components().any(|part| part.as_os_str() == "target"))
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "rs"))
        .map(walkdir::DirEntry::into_path)
        .collect()
}
