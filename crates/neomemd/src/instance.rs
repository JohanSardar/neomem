use std::fs::{self, File, TryLockError};
use std::os::unix::fs::PermissionsExt;

use anyhow::Context;
use neomem_proto::Home;

#[derive(Debug)]
pub struct InstanceLock {
    _held: File,
}

impl InstanceLock {
    pub fn acquire(home: &Home) -> anyhow::Result<Option<Self>> {
        let path = home.daemon_lock_path();
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _held: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error).context("locking the daemon lock file"),
        }
    }
}

pub fn prepare_home(home: &Home) -> anyhow::Result<()> {
    for dir in [home.root().to_path_buf(), home.run_dir(), home.spool_dir()] {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::set_permissions(home.root(), fs::Permissions::from_mode(0o700))
        .with_context(|| format!("restricting {}", home.root().display()))
}
