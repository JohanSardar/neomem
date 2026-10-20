use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use neomem_core::env_file_entries;

#[derive(Clone, Default)]
pub struct Secrets {
    path: Option<PathBuf>,
    from_file: BTreeMap<String, String>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Secrets")
            .field("path", &self.path)
            .field("names", &self.from_file.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Secrets {
    pub fn load(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        Self { path: Some(path.to_path_buf()), from_file: env_file_entries(&text) }
    }

    pub fn get(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| self.from_file.get(name).cloned())
    }

    pub fn store(&self, values: &[(&str, &str)]) -> io::Result<()> {
        let path = self.path.as_deref().ok_or_else(|| io::Error::other("these secrets have no file"))?;
        let current = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let replaced =
            |line: &str| env_file_entries(line).keys().any(|name| values.iter().any(|(key, _)| key == name));
        let kept = current.lines().filter(|line| !replaced(line)).map(str::to_owned);
        let added = values.iter().map(|(name, value)| format!("{name}={value}"));
        let text: String = kept.chain(added).flat_map(|line| [line, "\n".to_owned()]).collect();
        let staging = path.with_extension("env.writing");
        let mut file =
            fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&staging)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&staging, path)
    }
}

#[cfg(test)]
mod tests {
    use super::Secrets;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn reads_plain_and_exported_lines_and_never_prints_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        std::fs::write(&path, "# keys\nNEOMEM_TEST_ONE=alpha\nexport NEOMEM_TEST_TWO=\"beta\"\n").unwrap();
        let secrets = Secrets::load(&path);
        assert_eq!(secrets.get("NEOMEM_TEST_TWO").as_deref(), Some("beta"));
        assert!(!format!("{secrets:?}").contains("alpha"));
    }

    #[test]
    fn storing_replaces_named_lines_keeps_the_rest_and_stays_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        std::fs::write(&path, "# keys\nNEOMEM_TEST_KEEP=one\nexport NEOMEM_TEST_SWAP=old\n").unwrap();
        Secrets::load(&path).store(&[("NEOMEM_TEST_SWAP", "new"), ("NEOMEM_TEST_ADD", "two")]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# keys\nNEOMEM_TEST_KEEP=one\nNEOMEM_TEST_SWAP=new\nNEOMEM_TEST_ADD=two\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
