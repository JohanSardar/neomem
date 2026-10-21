use neomem_core::{EpochMs, IntakeId};
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
    pub fn last_run_started(&self, kind: &str) -> Result<Option<EpochMs>, StateError> {
        self.read(|connection| {
            let started: Option<i64> =
                connection.query_row("SELECT max(started_at) FROM runs WHERE kind = ?1", [kind], |row| {
                    row.get(0)
                })?;
            Ok(started.map(EpochMs::from_millis))
        })
    }

    pub fn pending_rechecks(&self) -> Result<Vec<IntakeId>, StateError> {
        self.read(|connection| {
            let mut statement = connection.prepare(
                "SELECT intake_id FROM jobs WHERE kind = 'recheck' AND state = 'pending' ORDER BY id",
            )?;
            let ids: Vec<String> = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
            ids.iter()
                .map(|id| {
                    id.parse().map_err(|error: neomem_core::InvalidId| StateError::Corrupt(error.to_string()))
                })
                .collect()
        })
    }

    pub fn finish_rechecks(&self, at: EpochMs) -> Result<(), StateError> {
        self.writer()?.execute(
            "UPDATE jobs SET state = 'done', finished_at = ?1 WHERE kind = 'recheck' AND state = 'pending'",
            [at.as_millis()],
        )?;
        Ok(())
    }

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
