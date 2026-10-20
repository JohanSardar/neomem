use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use neomem_proto::{Client, Home, Request};

use crate::binaries::Binaries;
use crate::isolated::{self, IsolatedNeomem, Models};
use crate::repo;

pub fn up(models: Models, home: Option<&Path>) -> anyhow::Result<bool> {
    let root = repo::root()?;
    let binaries = Binaries::build_fresh(&root)?;
    let isolated = match home {
        Some(home) => IsolatedNeomem::start_at_with(home, &binaries, models)?,
        None => IsolatedNeomem::start_with(&binaries, models)?,
    };
    if models == Models::Real && !isolated.has_live_key("JEV_API_KEY") {
        println!("neomemctl: no JEV_API_KEY in ~/.neomem/secrets.env, so the judge has no live Jev");
    }
    let kept = isolated.detach();
    remember_current_home(&root, &kept)?;
    println!("neomemctl: isolated neomem running with {models:?} models");
    println!("export NEOMEM_HOME={}", kept.display());
    Ok(true)
}

pub fn down(home: Option<&Path>) -> anyhow::Result<bool> {
    let home = resolve_home(home)?;
    match Client::connect(&home, std::time::Duration::from_secs(5)) {
        Ok(mut client) => {
            client.call(Request::Shutdown)?;
            println!("neomemctl: stopped the daemon at {}", home.root().display());
        }
        Err(_) => println!("neomemctl: no daemon is serving {}", home.root().display()),
    }
    stop_fake_models(&home);
    Ok(true)
}

pub fn logs(home: Option<&Path>) -> anyhow::Result<bool> {
    let home = resolve_home(home)?;
    let log = fs::read_to_string(home.daemon_log_path())
        .with_context(|| format!("reading {}", home.daemon_log_path().display()))?;
    print!("{log}");
    Ok(true)
}

fn resolve_home(home: Option<&Path>) -> anyhow::Result<Home> {
    if let Some(home) = home {
        return Ok(Home::at(home));
    }
    let remembered = fs::read_to_string(current_home_file(&repo::root()?))
        .context("no isolated neomem is running; start one with neomemctl up")?;
    Ok(Home::at(remembered.trim()))
}

fn remember_current_home(root: &Path, home: &Path) -> anyhow::Result<()> {
    let file = current_home_file(root);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, home.display().to_string())?;
    Ok(())
}

fn current_home_file(root: &Path) -> PathBuf {
    root.join("target/neomemctl/current-home")
}

fn stop_fake_models(home: &Home) {
    let Ok(pid) = fs::read_to_string(isolated::fake_models_pid_path(home)) else { return };
    let _ = std::process::Command::new("kill").arg(pid.trim()).status();
}
