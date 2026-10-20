use neomem_core::{EpochMs, IntakeId};
use rusqlite::{OptionalExtension, params};

use crate::{StateDb, StateError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionOutcome {
    Recorded,
    UnknownIntake,
}

impl StateDb {
    pub fn correct(
        &self,
        intake: IntakeId,
        reason: &str,
        at: EpochMs,
    ) -> Result<CorrectionOutcome, StateError> {
        let id = intake.to_string();
        let mut writer = self.writer()?;
        let transaction = writer.transaction()?;
        let known: Option<i64> = transaction
            .query_row("SELECT 1 FROM intake WHERE id = ?1", [&id], |row| row.get(0))
            .optional()?;
        if known.is_none() {
            return Ok(CorrectionOutcome::UnknownIntake);
        }
        transaction.execute(
            "INSERT INTO corrections (intake_id, reason, recorded_at) VALUES (?1, ?2, ?3)",
            params![id, reason, at.as_millis()],
        )?;
        transaction.execute(
            "UPDATE jobs SET state = 'skipped', finished_at = ?2 WHERE kind = 'extract' AND intake_id = ?1 AND state = 'pending'",
            params![id, at.as_millis()],
        )?;
        transaction.execute(
            "INSERT INTO jobs (kind, batch_key, intake_id, state, not_before, created_at)
             VALUES ('recheck', ?1, ?1, 'pending', ?2, ?2)",
            params![id, at.as_millis()],
        )?;
        transaction.commit()?;
        Ok(CorrectionOutcome::Recorded)
    }

    pub fn is_corrected(&self, intake: IntakeId) -> Result<bool, StateError> {
        self.read(|connection| {
            Ok(connection
                .query_row(
                    "SELECT 1 FROM corrections WHERE intake_id = ?1 LIMIT 1",
                    [intake.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .is_some())
        })
    }
}
