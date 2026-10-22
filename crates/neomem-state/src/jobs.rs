use std::fmt;

use neomem_core::{EpochMs, IntakeId, LoggedIntake, Source};
use rusqlite::{OptionalExtension, params};

use crate::{StateDb, StateError};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BatchKey(String);

impl BatchKey {
    pub fn for_record(logged: &LoggedIntake) -> Self {
        let record = &logged.record;
        match &record.session {
            Some(session) => Self::for_session(record.source.as_str(), session),
            None => Self(record.source.to_string()),
        }
    }

    pub fn for_session(source: &str, session: &str) -> Self {
        Self(format!("{source}/{session}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BatchKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionJob {
    pub batch: BatchKey,
    pub not_before: EpochMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedBatch {
    pub batch: BatchKey,
    pub jobs: Vec<(JobId, IntakeId)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    Done,
    RetryAfter(EpochMs),
    GaveUp,
}

const READY_BATCH: &str = "SELECT batch_key FROM jobs
    WHERE kind = 'extract' AND state = 'pending'
    GROUP BY batch_key
    HAVING max(not_before) <= ?1
    ORDER BY min(created_at)
    LIMIT 1";

const CLAIM_OLDEST_RECORDS: &str =
    "UPDATE jobs SET state = 'claimed', lease_until = ?2, attempts = attempts + 1
    WHERE id IN (
        SELECT id FROM (
            SELECT jobs.id AS id,
                sum(length(coalesce(intake.content, ''))) OVER (ORDER BY jobs.id)
                    - length(coalesce(intake.content, '')) AS earlier_characters
            FROM jobs LEFT JOIN intake ON intake.id = jobs.intake_id
            WHERE jobs.kind = 'extract' AND jobs.state = 'pending' AND jobs.batch_key = ?1
        )
        WHERE earlier_characters < ?3
    )
    RETURNING id, intake_id";

impl StateDb {
    pub fn extraction_counts(&self) -> Result<(usize, usize), StateError> {
        self.read(|connection| {
            let pending: i64 = connection.query_row(
                "SELECT count(*) FROM jobs WHERE kind = 'extract' AND state IN ('pending', 'claimed')",
                [],
                |row| row.get(0),
            )?;
            let failed: i64 = connection.query_row(
                "SELECT count(*) FROM jobs WHERE kind = 'extract' AND state = 'failed'",
                [],
                |row| row.get(0),
            )?;
            Ok((
                usize::try_from(pending).unwrap_or(usize::MAX),
                usize::try_from(failed).unwrap_or(usize::MAX),
            ))
        })
    }

    pub fn pending_in_batch(&self, batch: &BatchKey) -> Result<usize, StateError> {
        self.read(|connection| {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM jobs WHERE kind = 'extract' AND state = 'pending' AND batch_key = ?1",
                [batch.as_str()],
                |row| row.get(0),
            )?;
            Ok(usize::try_from(count).unwrap_or(0))
        })
    }

    pub fn hasten_batch(&self, batch: &BatchKey, now: EpochMs) -> Result<(), StateError> {
        self.writer()?.execute(
            "UPDATE jobs SET not_before = ?2 WHERE kind = 'extract' AND state = 'pending' AND batch_key = ?1",
            params![batch.as_str(), now.as_millis()],
        )?;
        Ok(())
    }

    pub fn claim_ready_batch(
        &self,
        now: EpochMs,
        lease_until: EpochMs,
        most_characters: usize,
    ) -> Result<Option<ClaimedBatch>, StateError> {
        let mut writer = self.writer()?;
        let transaction = writer.transaction()?;
        let key: Option<String> =
            transaction.query_row(READY_BATCH, [now.as_millis()], |row| row.get(0)).optional()?;
        let Some(key) = key else { return Ok(None) };
        let jobs = {
            let mut statement = transaction.prepare(CLAIM_OLDEST_RECORDS)?;
            let most_characters = i64::try_from(most_characters).unwrap_or(i64::MAX);
            let rows = statement
                .query_map(params![key, lease_until.as_millis(), most_characters], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
            let mut jobs = Vec::new();
            for row in rows {
                let (id, intake) = row?;
                let intake =
                    intake.parse::<IntakeId>().map_err(|error| StateError::Corrupt(error.to_string()))?;
                jobs.push((JobId(id), intake));
            }
            jobs.sort();
            jobs
        };
        transaction.commit()?;
        Ok(Some(ClaimedBatch { batch: BatchKey(key), jobs }))
    }

    pub fn finish_jobs(&self, jobs: &[JobId], outcome: JobOutcome, at: EpochMs) -> Result<(), StateError> {
        let mut writer = self.writer()?;
        let transaction = writer.transaction()?;
        for job in jobs {
            match outcome {
                JobOutcome::Done => transaction.execute(
                    "UPDATE jobs SET state = 'done', finished_at = ?2, lease_until = NULL WHERE id = ?1",
                    params![job.0, at.as_millis()],
                )?,
                JobOutcome::GaveUp => transaction.execute(
                    "UPDATE jobs SET state = 'failed', finished_at = ?2, lease_until = NULL WHERE id = ?1",
                    params![job.0, at.as_millis()],
                )?,
                JobOutcome::RetryAfter(not_before) => transaction.execute(
                    "UPDATE jobs SET state = 'pending', not_before = ?2, lease_until = NULL WHERE id = ?1",
                    params![job.0, not_before.as_millis()],
                )?,
            };
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn attempts(&self, job: JobId) -> Result<u32, StateError> {
        self.read(|connection| {
            let attempts: i64 =
                connection.query_row("SELECT attempts FROM jobs WHERE id = ?1", [job.0], |row| row.get(0))?;
            Ok(u32::try_from(attempts).unwrap_or(u32::MAX))
        })
    }

    pub fn requeue_extraction(
        &self,
        source: Option<&Source>,
        from: Option<EpochMs>,
        now: EpochMs,
    ) -> Result<usize, StateError> {
        let queued = self.writer()?.execute(
            "INSERT INTO jobs (kind, batch_key, intake_id, state, not_before, created_at)
             SELECT 'extract',
                    CASE WHEN session IS NULL THEN source ELSE source || '/' || session END,
                    id, 'pending', ?3, ?3
             FROM intake
             WHERE (?1 IS NULL OR source = ?1)
               AND (?2 IS NULL OR occurred_end >= ?2)
               AND NOT EXISTS (SELECT 1 FROM corrections c WHERE c.intake_id = intake.id)
               AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.intake_id = intake.id AND j.kind = 'extract' AND j.state IN ('pending', 'claimed'))",
            params![source.map(Source::as_str), from.map(EpochMs::as_millis), now.as_millis()],
        )?;
        Ok(queued)
    }

    pub fn reclaim_expired_leases(&self, now: EpochMs) -> Result<usize, StateError> {
        let changed = self.writer()?.execute(
            "UPDATE jobs SET state = 'pending', lease_until = NULL WHERE state = 'claimed' AND lease_until < ?1",
            [now.as_millis()],
        )?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_core::{IntakeRecord, Source, Span};

    fn log(state: &StateDb, seed: u128, session: Option<&str>, recorded: i64, not_before: i64) -> IntakeId {
        let logged = LoggedIntake {
            id: IntakeId::from_parts(EpochMs::from_millis(recorded), seed),
            recorded_at: EpochMs::from_millis(recorded),
            record: IntakeRecord {
                source: Source::new("claude-code").unwrap(),
                occurred: Span::instant(EpochMs::from_millis(recorded)),
                tags: vec![],
                content: format!("turn {seed}"),
                entity_hints: vec![],
                source_ref: None,
                session: session.map(str::to_owned),
            },
        };
        let job = ExtractionJob {
            batch: BatchKey::for_record(&logged),
            not_before: EpochMs::from_millis(not_before),
        };
        state.log_intake(&logged, &job).unwrap();
        logged.id
    }

    fn at(millis: i64) -> EpochMs {
        EpochMs::from_millis(millis)
    }

    #[test]
    fn a_session_is_ready_only_after_its_last_turn_went_quiet() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        log(&state, 1, Some("s1"), 1_000, 301_000);
        log(&state, 2, Some("s1"), 200_000, 500_000);
        assert_eq!(state.claim_ready_batch(at(400_000), at(1_000_000), usize::MAX).unwrap(), None);
        let claimed = state.claim_ready_batch(at(500_000), at(1_000_000), usize::MAX).unwrap().unwrap();
        assert_eq!(claimed.batch.as_str(), "claude-code/s1");
        assert_eq!(claimed.jobs.len(), 2);
        assert_eq!(state.claim_ready_batch(at(600_000), at(1_000_000), usize::MAX).unwrap(), None);
    }

    #[test]
    fn an_expired_lease_puts_the_batch_back() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        log(&state, 1, Some("s2"), 1_000, 2_000);
        let claimed = state.claim_ready_batch(at(3_000), at(10_000), usize::MAX).unwrap().unwrap();
        assert_eq!(state.reclaim_expired_leases(at(9_000)).unwrap(), 0);
        assert_eq!(state.reclaim_expired_leases(at(11_000)).unwrap(), 1);
        let again = state.claim_ready_batch(at(12_000), at(20_000), usize::MAX).unwrap().unwrap();
        assert_eq!(again.jobs, claimed.jobs);
        state
            .finish_jobs(
                &again.jobs.iter().map(|job| job.0).collect::<Vec<_>>(),
                JobOutcome::Done,
                at(13_000),
            )
            .unwrap();
        assert_eq!(state.claim_ready_batch(at(99_000), at(100_000), usize::MAX).unwrap(), None);
    }

    #[test]
    fn hastening_a_batch_makes_it_ready_now() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        log(&state, 1, None, 1_000, 1_800_000);
        let batch = BatchKey("claude-code".to_owned());
        assert_eq!(state.pending_in_batch(&batch).unwrap(), 1);
        state.hasten_batch(&batch, at(5_000)).unwrap();
        assert!(state.claim_ready_batch(at(5_000), at(9_000), usize::MAX).unwrap().is_some());
    }

    #[test]
    fn a_long_session_is_claimed_oldest_records_first_up_to_the_character_budget() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        let first = log(&state, 1, Some("s3"), 1_000, 2_000);
        let second = log(&state, 2, Some("s3"), 1_100, 2_000);
        let third = log(&state, 3, Some("s3"), 1_200, 2_000);
        let claimed = state.claim_ready_batch(at(3_000), at(10_000), 10).unwrap().unwrap();
        assert_eq!(claimed.jobs.iter().map(|job| job.1).collect::<Vec<_>>(), vec![first, second]);
        let rest = state.claim_ready_batch(at(3_000), at(10_000), 10).unwrap().unwrap();
        assert_eq!(rest.jobs.iter().map(|job| job.1).collect::<Vec<_>>(), vec![third]);
        assert_eq!(state.claim_ready_batch(at(3_000), at(10_000), 1).unwrap(), None);
    }
}
