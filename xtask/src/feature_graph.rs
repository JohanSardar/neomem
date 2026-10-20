use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::violation::Violation;

pub fn check(root: &Path) -> anyhow::Result<Vec<Violation>> {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--package", "neomem-control", "--bin", "neomemctl", "--", "map", "--check"])
        .current_dir(root)
        .output()
        .context("running neomemctl map --check")?;
    if output.status.success() {
        return Ok(Vec::new());
    }
    let report = String::from_utf8_lossy(&output.stderr);
    let mut violations: Vec<Violation> = report
        .lines()
        .filter_map(|line| line.strip_prefix("[feature-graph] "))
        .map(|problem| Violation {
            check: "feature-graph",
            file: PathBuf::from(".agents/skills/neomem-control/feature-graph.yaml"),
            line: 1,
            message: problem.to_owned(),
        })
        .collect();
    if violations.is_empty() {
        violations.push(Violation {
            check: "feature-graph",
            file: PathBuf::from("crates/neomem-control"),
            line: 1,
            message: format!("neomemctl map --check failed: {}", report.trim()),
        });
    }
    Ok(violations)
}
