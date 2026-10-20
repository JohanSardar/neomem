use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use neomem_core::{EpochMs, Memory, MemoryId};

use crate::paths::{self, ENTITIES_DIR, FACTS_DIR};
use crate::{format, git};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("the vault could not read or write a file: {0}")]
    Io(#[from] std::io::Error),
    #[error("a memory file is malformed: {0}")]
    Format(String),
    #[error("git failed: {0}")]
    Git(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOrigin {
    OwnWrite,
    External,
}

#[derive(Debug, Clone, Copy)]
pub struct CommitPolicy {
    pub every: Duration,
    pub after_changes: usize,
}

#[derive(Debug, Default)]
struct Pending {
    changes: usize,
    since: Option<EpochMs>,
}

#[derive(Debug)]
pub struct Vault {
    root: PathBuf,
    locations: Mutex<HashMap<MemoryId, PathBuf>>,
    own_writes: Mutex<HashMap<PathBuf, u64>>,
    pending: Mutex<Pending>,
}

impl Vault {
    pub fn open(root: &Path) -> Result<Self, VaultError> {
        fs::create_dir_all(root.join(FACTS_DIR))?;
        fs::create_dir_all(root.join(ENTITIES_DIR))?;
        git::ensure_repository(root)?;
        let vault = Self {
            root: root.to_path_buf(),
            locations: Mutex::new(HashMap::new()),
            own_writes: Mutex::new(HashMap::new()),
            pending: Mutex::new(Pending::default()),
        };
        let found: HashMap<MemoryId, PathBuf> = vault
            .scan()
            .into_iter()
            .filter_map(Result::ok)
            .map(|(path, memory)| (memory.id(), path))
            .collect();
        *lock(&vault.locations) = found;
        Ok(vault)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put(&self, memory: &Memory, now: EpochMs) -> Result<PathBuf, VaultError> {
        let text = format::render(memory)?;
        let path = self.path_for(memory);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("md.tmp");
        fs::write(&temporary, &text)?;
        fs::rename(&temporary, &path)?;
        lock(&self.own_writes).insert(path.clone(), checksum(text.as_bytes()));
        lock(&self.locations).insert(memory.id(), path.clone());
        self.note_pending(now);
        Ok(path)
    }

    pub fn get(&self, id: MemoryId) -> Result<Option<Memory>, VaultError> {
        let location = lock(&self.locations).get(&id).cloned();
        match location {
            Some(path) => self.read(&path).map(Some),
            None => Ok(None),
        }
    }

    pub fn path_of(&self, id: MemoryId) -> Option<PathBuf> {
        lock(&self.locations).get(&id).cloned()
    }

    pub fn read(&self, path: &Path) -> Result<Memory, VaultError> {
        format::parse(&fs::read_to_string(path)?)
    }

    pub fn scan(&self) -> Vec<Result<(PathBuf, Memory), VaultError>> {
        let mut files = Vec::new();
        collect_markdown(&self.root.join(FACTS_DIR), &mut files);
        collect_markdown(&self.root.join(ENTITIES_DIR), &mut files);
        files.sort();
        files.into_iter().map(|path| self.read(&path).map(|memory| (path, memory))).collect()
    }

    pub fn is_memory_file(&self, path: &Path) -> bool {
        path.extension().is_some_and(|extension| extension == "md")
            && (path.starts_with(self.root.join(FACTS_DIR)) || path.starts_with(self.root.join(ENTITIES_DIR)))
    }

    pub fn notice_change(&self, path: &Path, now: EpochMs) -> ChangeOrigin {
        let current = fs::read(path).map(|bytes| checksum(&bytes)).ok();
        let own = lock(&self.own_writes).get(path).copied();
        if current.is_some() && current == own {
            return ChangeOrigin::OwnWrite;
        }
        if let Ok(memory) = self.read(path) {
            lock(&self.locations).insert(memory.id(), path.to_path_buf());
        }
        self.note_pending(now);
        ChangeOrigin::External
    }

    pub fn commit_pending(&self, now: EpochMs, policy: CommitPolicy) -> Result<Option<usize>, VaultError> {
        let mut pending = lock(&self.pending);
        let Some(since) = pending.since else { return Ok(None) };
        let waited = u128::try_from(now.millis_since(since)).unwrap_or(0);
        if pending.changes < policy.after_changes && waited < policy.every.as_millis() {
            return Ok(None);
        }
        let changes = pending.changes;
        let committed = git::commit_everything_tracked(&self.root, &format!("neomem: {changes} changes"))?;
        *pending = Pending::default();
        Ok(committed.then_some(changes))
    }

    fn note_pending(&self, now: EpochMs) {
        let mut pending = lock(&self.pending);
        pending.changes += 1;
        pending.since.get_or_insert(now);
    }

    fn path_for(&self, memory: &Memory) -> PathBuf {
        if let Some(existing) = self.path_of(memory.id()) {
            return existing;
        }
        match memory {
            Memory::Fact(fact) => paths::fact_path(&self.root, fact),
            Memory::Entity(entity) => {
                let locations = lock(&self.locations);
                let taken = |path: &Path| path.exists() || locations.values().any(|used| used == path);
                paths::entity_path(&self.root, entity, taken)
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn collect_markdown(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_core::{
        Durability, Entity, EntityId, Fact, FactId, Importance, IntakeId, Label, NewEntity, NewFact,
    };

    fn at(millis: i64) -> EpochMs {
        EpochMs::from_millis(millis)
    }

    fn fact(seed: u128) -> Fact {
        Fact::new(NewFact {
            id: FactId::from_parts(at(1_792_400_100_000), seed),
            content: format!("Fact number {seed}"),
            tags: vec![],
            about: vec![EntityId::from_parts(at(1), 1)],
            recorded_at: at(1_792_400_100_000),
            valid_at: at(1_792_400_100_000),
            importance: Importance::new(5).unwrap(),
            durability: Durability::Ordinary,
            sources: vec![IntakeId::from_parts(at(1), 2)],
            links: vec![],
        })
        .unwrap()
    }

    const EVERY_SECOND: CommitPolicy = CommitPolicy { every: Duration::from_secs(1), after_changes: 50 };

    #[test]
    fn put_then_get_returns_the_same_memory_from_a_dated_path() {
        let home = tempfile::tempdir().unwrap();
        let vault = Vault::open(home.path()).unwrap();
        let memory = Memory::Fact(fact(1));
        let path = vault.put(&memory, at(0)).unwrap();
        assert!(path.starts_with(home.path().join("facts/2026/10")));
        assert_eq!(vault.get(memory.id()).unwrap(), Some(memory.clone()));
        let reopened = Vault::open(home.path()).unwrap();
        assert_eq!(reopened.get(memory.id()).unwrap(), Some(memory));
    }

    #[test]
    fn its_own_writes_are_told_apart_from_hand_edits() {
        let home = tempfile::tempdir().unwrap();
        let vault = Vault::open(home.path()).unwrap();
        let path = vault.put(&Memory::Fact(fact(2)), at(0)).unwrap();
        assert_eq!(vault.notice_change(&path, at(1)), ChangeOrigin::OwnWrite);
        let edited = fs::read_to_string(&path).unwrap().replace("Fact number 2", "Fact number two");
        fs::write(&path, edited).unwrap();
        assert_eq!(vault.notice_change(&path, at(2)), ChangeOrigin::External);
    }

    #[test]
    fn commits_wait_for_the_interval_or_the_change_count() {
        let home = tempfile::tempdir().unwrap();
        let vault = Vault::open(home.path()).unwrap();
        vault.put(&Memory::Fact(fact(3)), at(0)).unwrap();
        assert_eq!(vault.commit_pending(at(500), EVERY_SECOND).unwrap(), None);
        assert_eq!(vault.commit_pending(at(1_000), EVERY_SECOND).unwrap(), Some(1));
        assert_eq!(vault.commit_pending(at(5_000), EVERY_SECOND).unwrap(), None);
    }

    #[test]
    fn two_entities_with_one_name_get_two_files() {
        let home = tempfile::tempdir().unwrap();
        let vault = Vault::open(home.path()).unwrap();
        let sam = |seed| {
            Memory::Entity(
                Entity::new(NewEntity {
                    id: EntityId::from_parts(at(1), seed),
                    kind: Label::new("person").unwrap(),
                    name: "Sam Kim".to_owned(),
                    aliases: vec![],
                    description: String::new(),
                    recorded_at: at(1),
                })
                .unwrap(),
            )
        };
        let first = vault.put(&sam(1), at(0)).unwrap();
        let second = vault.put(&sam(2), at(0)).unwrap();
        assert_ne!(first, second);
        assert!(first.ends_with("entities/person/sam-kim.md"));
    }
}
