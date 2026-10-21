use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

const BACKEND_ENVIRONMENT: &[(&str, &str)] = &[("GGML_VK_DISABLE_GRAPH_OPTIMIZE", "1")];

pub fn apply() -> io::Result<()> {
    if BACKEND_ENVIRONMENT.iter().all(|(name, _)| std::env::var_os(name).is_some()) {
        return Ok(());
    }
    let neomemd = std::env::current_exe()?;
    Err(Command::new(neomemd)
        .args(std::env::args_os().skip(1))
        .envs(BACKEND_ENVIRONMENT.iter().copied())
        .exec())
}
