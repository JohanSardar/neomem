use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

pub fn changed_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let base = comparison_base(root)?;
    let listing = git(root, &["diff", "--name-only", &base, "HEAD"])?;
    Ok(listing.lines().map(PathBuf::from).collect())
}

fn comparison_base(root: &Path) -> anyhow::Result<String> {
    let head = git(root, &["rev-parse", "HEAD"])?;
    let fork_point = git(root, &["merge-base", "origin/main", "HEAD"]).unwrap_or_default();
    if fork_point.is_empty() || fork_point == head {
        return Ok("HEAD^1".to_owned());
    }
    Ok(fork_point)
}

fn git(root: &Path, arguments: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!("git {} failed", arguments.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
