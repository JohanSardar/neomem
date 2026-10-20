use std::path::Path;
use std::process::Command;

use crate::VaultError;

const IGNORED: &str =
    "state.db\nstate.db-*\nindex.db\nindex.db-*\nmodels/\nrun/\nspool/\nlogs/\nsecrets.env\n*.env\n*.tmp\n";
const TRACKED: &[&str] = &["facts", "entities", "config.toml", ".gitignore"];

pub fn ensure_repository(root: &Path) -> Result<(), VaultError> {
    std::fs::write(root.join(".gitignore"), IGNORED)?;
    if !root.join(".git").exists() {
        run(root, &["init", "--quiet", "--initial-branch=main"])?;
    }
    run(root, &["config", "user.name", "neomem"])?;
    run(root, &["config", "user.email", "neomem@localhost"])?;
    run(root, &["config", "commit.gpgsign", "false"])?;
    Ok(())
}

pub fn commit_everything_tracked(root: &Path, message: &str) -> Result<bool, VaultError> {
    let present: Vec<&str> = TRACKED.iter().copied().filter(|path| root.join(path).exists()).collect();
    let mut add = vec!["add", "--all", "--"];
    add.extend(present);
    run(root, &add)?;
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(root)
        .status()
        .map_err(|error| VaultError::Git(error.to_string()))?;
    if staged.success() {
        return Ok(false);
    }
    run(root, &["-c", "core.hooksPath=/dev/null", "commit", "--quiet", "--no-verify", "-m", message])?;
    Ok(true)
}

fn run(root: &Path, arguments: &[&str]) -> Result<(), VaultError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| VaultError::Git(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(VaultError::Git(format!(
            "git {} failed: {}",
            arguments.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
