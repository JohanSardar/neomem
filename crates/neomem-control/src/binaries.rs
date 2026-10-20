use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

#[derive(Debug, Clone)]
pub struct Binaries {
    pub neomem: PathBuf,
    pub neomemd: PathBuf,
}

impl Binaries {
    pub fn beside_this_executable() -> anyhow::Result<Self> {
        let directory = std::env::current_exe()?
            .parent()
            .map(Path::to_path_buf)
            .context("neomemctl has no parent directory")?;
        Ok(Self { neomem: directory.join("neomem"), neomemd: directory.join("neomemd") })
    }

    pub fn build_fresh(root: &Path) -> anyhow::Result<Self> {
        let release = std::env::current_exe()?.components().any(|part| part.as_os_str() == "release");
        build(root, release)?;
        Self::beside_this_executable()
    }

    pub fn build_release(root: &Path) -> anyhow::Result<Self> {
        build(root, true)?;
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| root.join("target"), std::path::PathBuf::from);
        let directory = target.join("release");
        Ok(Self { neomem: directory.join("neomem"), neomemd: directory.join("neomemd") })
    }
}

fn build(root: &Path, release: bool) -> anyhow::Result<()> {
    let mut build = Command::new("cargo");
    build.args(["build", "--quiet", "--package", "neomemd", "--package", "neomem-cli"]).current_dir(root);
    if release {
        build.arg("--release");
    }
    if !build.status().context("running cargo build")?.success() {
        bail!("building neomemd and neomem failed");
    }
    Ok(())
}
