use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

const STEPS: &[(&str, &[&str])] = &[
    ("format", &["cargo", "fmt", "--all", "--check"]),
    ("build", &["cargo", "build", "--workspace", "--all-targets", "--locked"]),
    ("tests", &["cargo", "nextest", "run", "--workspace", "--no-tests=pass"]),
    (
        "clippy (pedantic)",
        &["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
    ),
    ("cargo-deny", &["cargo", "deny", "check"]),
    ("xtask check", &["cargo", "xtask", "check"]),
    ("prove changed features", &["target/debug/neomemctl", "prove", "--changed"]),
];

pub fn run_every_step(root: &Path) -> anyhow::Result<()> {
    for (name, command_line) in STEPS {
        eprintln!("ci: {name}");
        run_step(root, command_line).with_context(|| format!("ci step `{name}` failed"))?;
    }
    eprintln!("ci: every step passed");
    Ok(())
}

fn run_step(root: &Path, command_line: &[&str]) -> anyhow::Result<()> {
    let (program, arguments) = command_line.split_first().context("empty ci step")?;
    let status = Command::new(program).args(arguments).current_dir(root).status()?;
    if !status.success() {
        bail!("{} exited with {status}", command_line.join(" "));
    }
    Ok(())
}
