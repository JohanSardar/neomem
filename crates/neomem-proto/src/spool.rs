use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Envelope, Home, IncomingEnvelope};

const REJECTED_DIR: &str = "rejected";

#[derive(Debug)]
pub struct Spooled {
    pub path: PathBuf,
    pub envelope: Result<IncomingEnvelope, String>,
}

pub fn spool(home: &Home, envelope: &Envelope) -> io::Result<PathBuf> {
    let directory = home.spool_dir();
    fs::create_dir_all(&directory)?;
    let body = serde_json::to_vec(envelope).map_err(io::Error::other)?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let mut hasher = DefaultHasher::new();
    hasher.write(&body);
    hasher.write_u32(std::process::id());
    hasher.write_u128(nanos);
    let name = format!("{nanos:020}-{:016x}.json", hasher.finish());
    let temporary = directory.join(format!(".{name}.tmp"));
    fs::write(&temporary, &body)?;
    let path = directory.join(name);
    fs::rename(&temporary, &path)?;
    Ok(path)
}

pub fn spooled(home: &Home) -> Vec<Spooled> {
    let Ok(entries) = fs::read_dir(home.spool_dir()) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    paths.sort();
    paths.into_iter().map(|path| Spooled { envelope: read(&path), path }).collect()
}

pub fn set_aside(home: &Home, path: &Path) -> io::Result<PathBuf> {
    let rejected = home.spool_dir().join(REJECTED_DIR);
    fs::create_dir_all(&rejected)?;
    let target = rejected.join(path.file_name().unwrap_or_default());
    fs::rename(path, &target)?;
    Ok(target)
}

pub fn finish(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

fn read(path: &Path) -> Result<IncomingEnvelope, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Request;

    #[test]
    fn spooled_requests_come_back_in_order_and_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let home = Home::at(directory.path());
        let first = spool(&home, &Envelope::current(Request::Status)).unwrap();
        let second = spool(&home, &Envelope::current(Request::Status)).unwrap();
        assert_ne!(first, second);
        let found = spooled(&home);
        assert_eq!(found.iter().map(|item| item.path.clone()).collect::<Vec<_>>(), vec![first, second]);
        assert!(found.iter().all(|item| item.envelope.is_ok()));
    }

    #[test]
    fn unreadable_files_are_reported_and_can_be_set_aside() {
        let directory = tempfile::tempdir().unwrap();
        let home = Home::at(directory.path());
        fs::create_dir_all(home.spool_dir()).unwrap();
        let broken = home.spool_dir().join("broken.json");
        fs::write(&broken, "{").unwrap();
        assert!(spooled(&home)[0].envelope.is_err());
        let moved = set_aside(&home, &broken).unwrap();
        assert!(moved.exists() && spooled(&home).is_empty());
    }
}
