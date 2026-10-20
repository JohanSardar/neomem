use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::graph::{Feature, Status};

const TOOLING_CRATES: &[&str] = &["neomem-control", "neomem-eval", "neomem-testkit"];

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
    let output = Command::new("git").args(arguments).current_dir(root).output().context("running git")?;
    if !output.status.success() {
        bail!("git {} failed", arguments.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn affects(changed: &[PathBuf], feature: &Feature) -> bool {
    if feature.status == Status::Planned {
        return false;
    }
    let scenario_file = format!("crates/neomem-control/src/scenarios/{}.rs", feature.proof.replace('-', "_"));
    let own_crate = format!("crates/{}/", feature.crate_name());
    changed.iter().any(|file| {
        file.starts_with(&own_crate) || file == Path::new(&scenario_file) || touches_runtime_crate(file)
    })
}

fn touches_runtime_crate(file: &Path) -> bool {
    let mut parts = file.components().map(|part| part.as_os_str().to_string_lossy());
    match (parts.next().as_deref(), parts.next()) {
        (Some("crates"), Some(crate_name)) => !TOOLING_CRATES.contains(&crate_name.as_ref()),
        (Some("integrations"), Some(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_saphyr::FlowSeq;

    fn proven(id: &str, module: &str) -> Feature {
        Feature {
            id: id.to_owned(),
            does: String::new(),
            reached_via: FlowSeq(Vec::new()),
            module: module.to_owned(),
            depends_on: FlowSeq(Vec::new()),
            proof: id.replace('.', "-"),
            status: Status::Proven,
            last_proof: None,
            blocked: None,
        }
    }

    #[test]
    fn runtime_crate_changes_reprove_every_feature() {
        let changed = vec![PathBuf::from("crates/neomem-proto/src/lib.rs")];
        assert!(affects(&changed, &proven("raw", "neomem-state (raw)")));
    }

    #[test]
    fn a_scenario_change_reproves_only_its_feature() {
        let changed = vec![PathBuf::from("crates/neomem-control/src/scenarios/models_embed.rs")];
        assert!(affects(&changed, &proven("models.embed", "neomem-models (embed)")));
        assert!(!affects(&changed, &proven("daemon", "neomemd")));
    }
}
