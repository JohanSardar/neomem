use std::env;
use std::path::{Path, PathBuf};

const LONGEST_SOCKET_PATH: usize = if cfg!(target_os = "macos") { 103 } else { 107 };

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    root: PathBuf,
}

impl Home {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn from_env() -> Option<Self> {
        if let Some(explicit) = env::var_os("NEOMEM_HOME").filter(|value| !value.is_empty()) {
            return Some(Self::at(explicit));
        }
        env::var_os("HOME").map(|user_home| Self::at(Path::new(&user_home).join(".neomem")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }

    pub fn spool_dir(&self) -> PathBuf {
        self.root.join("spool")
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn secrets_path(&self) -> PathBuf {
        self.root.join("secrets.env")
    }

    pub fn state_db_path(&self) -> PathBuf {
        self.root.join("state.db")
    }

    pub fn index_db_path(&self) -> PathBuf {
        self.root.join("index.db")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn daemon_lock_path(&self) -> PathBuf {
        self.run_dir().join("neomemd.lock")
    }

    pub fn daemon_log_path(&self) -> PathBuf {
        self.run_dir().join("neomemd.log")
    }

    pub fn http_address_path(&self) -> PathBuf {
        self.run_dir().join("neomemd.http")
    }

    pub fn socket_path(&self) -> PathBuf {
        let preferred = self.run_dir().join("neomemd.sock");
        if preferred.as_os_str().len() <= LONGEST_SOCKET_PATH {
            return preferred;
        }
        let short_dir = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir())
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        short_dir.join(format!("neomem-{:016x}.sock", fingerprint(self.root.as_os_str().as_encoded_bytes())))
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_home_keeps_its_socket_in_run() {
        let home = Home::at("/tmp/neomem-short");
        assert_eq!(home.socket_path(), PathBuf::from("/tmp/neomem-short/run/neomemd.sock"));
    }

    #[test]
    fn a_long_home_moves_its_socket_somewhere_short_and_stable() {
        let home = Home::at(format!("/tmp/{}", "deep/".repeat(30)));
        let socket = home.socket_path();
        assert!(socket.as_os_str().len() <= LONGEST_SOCKET_PATH);
        assert_eq!(socket, home.socket_path());
        assert_ne!(socket, Home::at("/tmp/other").socket_path());
    }
}
