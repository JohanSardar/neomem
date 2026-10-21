use neomem_core::{EpochMs, IntakeId, IntakeRecord, LoggedIntake, Source, Span, Tag};
use rusqlite::{OptionalExtension, Row, params};

use crate::{ExtractionJob, StateDb, StateError};

const INTAKE_COLUMNS: &str =
    "id, recorded_at, source, occurred_start, occurred_end, tags, content, entity_hints, source_ref, session";

impl StateDb {
    pub fn log_intake(&self, logged: &LoggedIntake, job: &ExtractionJob) -> Result<(), StateError> {
        let mut writer = self.writer()?;
        let transaction = writer.transaction()?;
        insert_intake(&transaction, logged, job)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn intake(&self, id: IntakeId) -> Result<Option<LoggedIntake>, StateError> {
        self.read(|connection| {
            connection
                .query_row(
                    &format!("SELECT {INTAKE_COLUMNS} FROM intake WHERE id = ?1"),
                    [id.to_string()],
                    |row| Ok(logged_from(row)),
                )
                .optional()?
                .transpose()
        })
    }
}

impl StateDb {
    pub fn session_turns_before(
        &self,
        source: &Source,
        session: &str,
        before: EpochMs,
        limit: usize,
    ) -> Result<Vec<LoggedIntake>, StateError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.read(|connection| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {INTAKE_COLUMNS} FROM intake WHERE source = ?1 AND session = ?2 AND recorded_at < ?3
                 ORDER BY recorded_at DESC LIMIT ?4"
            ))?;
            let rows = statement
                .query_map(params![source.as_str(), session, before.as_millis(), limit], |row| {
                    Ok(logged_from(row))
                })?;
            let mut turns =
                rows.collect::<Result<Vec<_>, _>>()?.into_iter().collect::<Result<Vec<_>, _>>()?;
            turns.reverse();
            Ok(turns)
        })
    }
}

pub(crate) fn insert_intake(
    transaction: &rusqlite::Transaction<'_>,
    logged: &LoggedIntake,
    job: &ExtractionJob,
) -> Result<(), StateError> {
    let record = &logged.record;
    transaction.execute(
        &format!("INSERT INTO intake ({INTAKE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
        params![
            logged.id.to_string(),
            logged.recorded_at.as_millis(),
            record.source.as_str(),
            record.occurred.start.as_millis(),
            record.occurred.end.as_millis(),
            to_json(&record.tags)?,
            record.content,
            to_json(&record.entity_hints)?,
            record.source_ref,
            record.session,
        ],
    )?;
    transaction.execute(
        "INSERT INTO jobs (kind, batch_key, intake_id, state, not_before, created_at)
             VALUES ('extract', ?1, ?2, 'pending', ?3, ?4)",
        params![
            job.batch.as_str(),
            logged.id.to_string(),
            job.not_before.as_millis(),
            logged.recorded_at.as_millis()
        ],
    )?;
    Ok(())
}

pub(crate) fn logged_from(row: &Row<'_>) -> Result<LoggedIntake, StateError> {
    let text = |index: usize| -> Result<String, StateError> { Ok(row.get::<_, String>(index)?) };
    let millis = |index: usize| -> Result<EpochMs, StateError> { Ok(EpochMs::from_millis(row.get(index)?)) };
    let id = text(0)?.parse::<IntakeId>().map_err(|error| StateError::Corrupt(error.to_string()))?;
    let source = Source::new(&text(2)?).map_err(|error| StateError::Corrupt(error.to_string()))?;
    let tags: Vec<Tag> = from_json(&text(5)?)?;
    let entity_hints: Vec<String> = from_json(&text(7)?)?;
    Ok(LoggedIntake {
        id,
        recorded_at: millis(1)?,
        record: IntakeRecord {
            source,
            occurred: Span { start: millis(3)?, end: millis(4)? },
            tags,
            content: text(6)?,
            entity_hints,
            source_ref: row.get(8)?,
            session: row.get(9)?,
        },
    })
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, StateError> {
    serde_json::to_string(value).map_err(|error| StateError::Corrupt(error.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StateError> {
    serde_json::from_str(text).map_err(|error| StateError::Corrupt(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BatchKey;

    fn sample(id_seed: u128) -> LoggedIntake {
        LoggedIntake {
            id: IntakeId::from_parts(EpochMs::from_millis(1_760_000_000_000), id_seed),
            recorded_at: EpochMs::from_millis(1_760_000_000_000),
            record: IntakeRecord {
                source: Source::new("oura-digest").unwrap(),
                occurred: Span { start: EpochMs::from_millis(1), end: EpochMs::from_millis(2) },
                tags: vec![Tag::new("sleep").unwrap()],
                content: "Slept 7h".to_owned(),
                entity_hints: vec!["Oura".to_owned()],
                source_ref: Some("oura://1".to_owned()),
                session: None,
            },
        }
    }

    #[test]
    fn a_logged_record_reads_back_identically() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDb::open(&dir.path().join("state.db")).unwrap();
        let logged = sample(1);
        let job = ExtractionJob { batch: BatchKey::for_record(&logged), not_before: logged.recorded_at };
        state.log_intake(&logged, &job).unwrap();
        assert_eq!(state.intake(logged.id).unwrap(), Some(logged));
    }

    #[test]
    fn the_same_id_cannot_be_logged_twice() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDb::open(&dir.path().join("state.db")).unwrap();
        let logged = sample(2);
        let job = ExtractionJob { batch: BatchKey::for_record(&logged), not_before: logged.recorded_at };
        state.log_intake(&logged, &job).unwrap();
        assert!(state.log_intake(&logged, &job).is_err());
    }
}
