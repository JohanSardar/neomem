use std::path::{Path, PathBuf};

use neomem_vault::ChangeOrigin;

use crate::Engine;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    FileChanged(PathBuf),
}

impl Engine {
    pub fn notice(&self, notice: Notice) {
        match notice {
            Notice::FileChanged(path) if self.vault.is_memory_file(&path) => self.vault_file_changed(&path),
            Notice::FileChanged(_) => {}
        }
    }

    fn vault_file_changed(&self, path: &Path) {
        let now = self.clock.now();
        if !path.exists() || self.vault.notice_change(path, now) == ChangeOrigin::OwnWrite {
            return;
        }
        if self.vault.read(path).is_err() {
            let subject = file_subject(path);
            tracing::warn!("vault file {subject} is malformed and was not indexed");
            if let Err(error) = self.state.log_error(now, "vault_unreadable", &subject) {
                tracing::error!("recording a vault error failed: {error}");
            }
        }
    }
}

fn file_subject(path: &Path) -> String {
    let name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    name.split('_').take(2).collect::<Vec<_>>().join("_")
}
