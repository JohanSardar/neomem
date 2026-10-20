use neomem_core::EpochMs;
use rusqlite::params;

use crate::{StateDb, StateError};

impl StateDb {
    pub fn log_error(&self, at: EpochMs, kind: &str, subject: &str) -> Result<(), StateError> {
        self.writer()?.execute(
            "INSERT INTO errors (at, kind, subject) VALUES (?1, ?2, ?3)",
            params![at.as_millis(), kind, subject],
        )?;
        Ok(())
    }
}
