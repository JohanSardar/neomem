use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::gpu_build;

#[derive(Debug, Clone)]
pub struct Binaries {
    pub neomem: PathBuf,
    pub neomemd: PathBuf,
    pub fake_llm: PathBuf,
}

impl Binaries {
    pub fn beside_this_executable() -> anyhow::Result<Self> {
        let directory = std::env::current_exe()?
            .parent()
            .map(Path::to_path_buf)
            .context("neomemctl has no parent directory")?;
        Ok(Self::in_directory(&directory))
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
        Ok(Self::in_directory(&target.join("release")))
    }

    fn in_directory(directory: &Path) -> Self {
        Self {
            neomem: directory.join("neomem"),
            neomemd: directory.join("neomemd"),
            fake_llm: directory.join("neomem-fake-llm"),
        }
    }
}

fn build(root: &Path, release: bool) -> anyhow::Result<()> {
    let mut build = Command::new("cargo");
    build
        .args(["build", "--quiet", "--bins", "--package", "neomemd", "--package", "neomem-cli"])
        .args(["--package", "neomem-testkit"])
        .current_dir(root);
    if release {
        build.arg("--release");
    }
    if let Some(gpu) = gpu_build::choose()? {
        build.args(["--features", gpu.feature]).envs(gpu.environment);
    }
    if !build.status().context("running cargo build")?.success() {
        bail!("building neomemd, neomem and the fake LLM processes failed");
    }
    Ok(())
}
