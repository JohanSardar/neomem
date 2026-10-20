use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::repo;

pub fn fixture_dir(name: &str) -> anyhow::Result<PathBuf> {
    let directory = repo::root()?.join(".agents/skills/neomem-control/fixtures").join(name);
    if !directory.is_dir() {
        bail!("there is no fixture named {name}");
    }
    Ok(directory)
}

pub fn copy_vault_fixture(name: &str, home: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let source = fixture_dir(&format!("vault/{name}"))?;
    let mut copied = Vec::new();
    copy_tree(&source, &source, home, &mut copied)?;
    Ok(copied)
}

fn copy_tree(base: &Path, directory: &Path, home: &Path, copied: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            copy_tree(base, &path, home, copied)?;
            continue;
        }
        let relative = path.strip_prefix(base)?;
        let target = home.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&path, &target).with_context(|| format!("copying {}", relative.display()))?;
        copied.push(relative.to_path_buf());
    }
    Ok(())
}
