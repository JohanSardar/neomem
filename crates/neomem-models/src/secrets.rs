use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use neomem_core::env_file_entries;

#[derive(Clone, Default)]
pub struct Secrets {
    from_file: BTreeMap<String, String>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Secrets").field("names", &self.from_file.keys().collect::<Vec<_>>()).finish()
    }
}

impl Secrets {
    pub fn load(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        Self { from_file: env_file_entries(&text) }
    }

    pub fn get(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| self.from_file.get(name).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::Secrets;

    #[test]
    fn reads_plain_and_exported_lines_and_never_prints_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        std::fs::write(&path, "# keys\nNEOMEM_TEST_ONE=alpha\nexport NEOMEM_TEST_TWO=\"beta\"\n").unwrap();
        let secrets = Secrets::load(&path);
        assert_eq!(secrets.get("NEOMEM_TEST_TWO").as_deref(), Some("beta"));
        assert!(!format!("{secrets:?}").contains("alpha"));
    }
}
