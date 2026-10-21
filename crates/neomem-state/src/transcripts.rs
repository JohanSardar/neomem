use neomem_core::{EpochMs, LoggedIntake};
use rusqlite::{OptionalExtension, params};

use crate::{BatchKey, ExtractionJob, StateDb, StateError};

#[derive(Debug)]
pub struct TranscriptPosition<'a> {
    pub agent: &'a str,
    pub path: &'a str,
    pub session: &'a str,
    pub position: u64,
}

impl StateDb {
    pub fn transcript_paths(
        &self,
        agent: &str,
        session: &str,
    ) -> Result<Vec<std::path::PathBuf>, StateError> {
        self.read(|connection| {
            let mut statement = connection.prepare_cached(
                "SELECT path FROM transcript_positions WHERE agent = ?1 AND session = ?2 ORDER BY path",
            )?;
            let rows = statement.query_map(params![agent, session], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?.into_iter().map(std::path::PathBuf::from).collect())
        })
    }

    pub fn transcript_position(&self, agent: &str, path: &str) -> Result<u64, StateError> {
        self.read(|connection| {
            connection
                .query_row(
                    "SELECT position FROM transcript_positions WHERE agent = ?1 AND path = ?2",
                    params![agent, path],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0)
                .try_into()
                .map_err(|_| StateError::Corrupt("negative transcript position".to_owned()))
        })
    }

    pub fn log_transcript(
        &self,
        cursor: &TranscriptPosition<'_>,
        records: &[LoggedIntake],
        not_before: EpochMs,
    ) -> Result<(), StateError> {
        let TranscriptPosition { agent, path, session, position } = cursor;
        let position = i64::try_from(*position).map_err(|error| StateError::Corrupt(error.to_string()))?;
        let mut writer = self.writer()?;
        let transaction = writer.transaction()?;
        for logged in records {
            if let Some(turn) = &logged.record.source_ref
                && transaction.execute(
                    "INSERT OR IGNORE INTO transcript_entries (agent, path, turn) VALUES (?1, ?2, ?3)",
                    params![agent, path, turn],
                )? == 0
            {
                continue;
            }

            let job = ExtractionJob { batch: BatchKey::for_record(logged), not_before };
            super::intake::insert_intake(&transaction, logged, &job)?;
        }
        transaction.execute("INSERT INTO transcript_positions (agent, path, session, position) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(agent, path) DO UPDATE SET session = excluded.session, position = excluded.position", params![agent, path, session, position])?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_core::{IntakeId, IntakeRecord, Source, Span};

    fn record(seed: u128) -> LoggedIntake {
        let now = EpochMs::from_millis(100);
        LoggedIntake {
            id: IntakeId::from_parts(now, seed),
            recorded_at: now,
            record: IntakeRecord {
                source: Source::new("claude-code").unwrap(),
                occurred: Span::instant(now),
                tags: Vec::new(),
                content: "message".to_owned(),
                entity_hints: Vec::new(),
                source_ref: None,
                session: Some("session".to_owned()),
            },
        }
    }

    #[test]
    fn transcript_batch_and_position_roll_back_together() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        let first = record(1);
        let second = record(2);
        let mut cursor =
            TranscriptPosition { agent: "claude-code", path: "transcript", session: "session", position: 42 };
        state.log_transcript(&cursor, std::slice::from_ref(&first), first.recorded_at).unwrap();
        cursor.position = 100;
        assert!(state.log_transcript(&cursor, &[second.clone(), first], second.recorded_at).is_err());
        assert_eq!(state.transcript_position(cursor.agent, cursor.path).unwrap(), 42);
        assert!(state.intake(second.id).unwrap().is_none());
    }

    #[test]
    fn byte_positions_survive_beyond_two_gigabytes() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        let cursor = TranscriptPosition {
            agent: "claude-code",
            path: "large",
            session: "session",
            position: 3_000_000_000,
        };
        state.log_transcript(&cursor, &[], EpochMs::from_millis(100)).unwrap();
        assert_eq!(state.transcript_position(cursor.agent, cursor.path).unwrap(), cursor.position);
    }
    #[test]
    fn revisited_turns_are_checkpointed_but_distinct_turns_are_not_deduplicated() {
        let directory = tempfile::tempdir().unwrap();
        let state = StateDb::open(&directory.path().join("state.db")).unwrap();
        let mut first = record(1);
        first.record.source_ref = Some("transcript#0".to_owned());
        let mut repeated = record(2);
        repeated.record.source_ref = first.record.source_ref.clone();
        let mut distinct = record(3);
        distinct.record.source_ref = Some("transcript#100".to_owned());
        let mut cursor = TranscriptPosition {
            agent: "claude-code",
            path: "transcript",
            session: "session",
            position: 100,
        };
        state.log_transcript(&cursor, std::slice::from_ref(&first), first.recorded_at).unwrap();
        cursor.position = 200;
        state.log_transcript(&cursor, &[repeated.clone(), distinct.clone()], first.recorded_at).unwrap();
        assert!(state.intake(first.id).unwrap().is_some());
        assert!(state.intake(repeated.id).unwrap().is_none());
        assert!(state.intake(distinct.id).unwrap().is_some());
        assert_eq!(state.transcript_position(cursor.agent, cursor.path).unwrap(), 200);
    }
}
