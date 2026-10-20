use neomem_core::EpochMs;
use rusqlite::params;

use crate::{StateDb, StateError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord<'run> {
    pub kind: &'run str,
    pub started_at: EpochMs,
    pub finished_at: EpochMs,
    pub model: Option<&'run str>,
    pub changes: serde_json::Value,
}

impl StateDb {
    pub fn log_run(&self, run: &RunRecord<'_>) -> Result<(), StateError> {
        self.writer()?.execute(
            "INSERT INTO runs (kind, started_at, finished_at, model, changes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run.kind,
                run.started_at.as_millis(),
                run.finished_at.as_millis(),
                run.model,
                run.changes.to_string()
            ],
        )?;
        Ok(())
    }
}
