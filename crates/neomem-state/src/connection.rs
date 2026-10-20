use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::schema;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state.db failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("state.db holds a value that does not parse: {0}")]
    Corrupt(String),
    #[error("the state.db writer lock was poisoned by a panic")]
    Poisoned,
}

#[derive(Debug)]
pub struct StateDb {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

impl StateDb {
    pub fn open(path: &Path) -> Result<Self, StateError> {
        let mut writer = Connection::open(path)?;
        configure(&writer)?;
        schema::migrate(&mut writer)?;
        Ok(Self { path: path.to_path_buf(), writer: Mutex::new(writer), readers: Mutex::new(Vec::new()) })
    }

    pub fn open_read_only(path: &Path) -> Result<Self, StateError> {
        let reader = open_reader(path)?;
        Ok(Self { path: path.to_path_buf(), writer: Mutex::new(reader), readers: Mutex::new(Vec::new()) })
    }

    pub(crate) fn writer(&self) -> Result<MutexGuard<'_, Connection>, StateError> {
        self.writer.lock().map_err(|_| StateError::Poisoned)
    }

    pub(crate) fn read<T>(
        &self,
        query: impl FnOnce(&Connection) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        let pooled = self.readers.lock().map_err(|_| StateError::Poisoned)?.pop();
        let connection = match pooled {
            Some(connection) => connection,
            None => open_reader(&self.path)?,
        };
        let outcome = query(&connection);
        self.readers.lock().map_err(|_| StateError::Poisoned)?.push(connection);
        outcome
    }
}

fn configure(connection: &Connection) -> Result<(), StateError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn open_reader(path: &Path) -> Result<Connection, StateError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}
