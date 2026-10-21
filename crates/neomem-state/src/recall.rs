use crate::{StateDb, StateError};
use neomem_core::{EpochMs, FactId};
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct RecallScore {
    pub id: FactId,
    pub probability: Option<f32>,
}

#[derive(Debug, Default, Clone)]
pub struct RecallLog {
    pub gate: Option<f32>,
    pub queries: usize,
    pub source: String,
    pub candidates: Vec<RecallScore>,
    pub injected: Vec<FactId>,
    pub timings: Vec<(String, f64)>,
    pub models: Vec<String>,
}

impl StateDb {
    pub fn label_skipped_recall(
        &self,
        at: EpochMs,
        agent: &str,
        session: &str,
        turn: &str,
    ) -> Result<(), StateError> {
        self.writer()?.execute("INSERT OR IGNORE INTO recall_search_labels (recall_id, searched_at) SELECT id, ?1 FROM recall_log WHERE id = (SELECT id FROM recall_log WHERE agent = ?2 AND session = ?3 AND turn = ?4 ORDER BY id DESC LIMIT 1) AND gate < 0.5", params![at.as_millis(), agent, session, turn])?;
        Ok(())
    }

    pub fn was_injected(&self, agent: &str, session: &str, fact: FactId) -> Result<bool, StateError> {
        self.read(|connection| {
            Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM recall_injected WHERE agent = ?1 AND session = ?2 AND fact = ?3)",
            params![agent, session, fact.to_string()], |row| row.get(0),
        )?)
        })
    }

    pub fn finish_recall(
        &self,
        at: EpochMs,
        agent: &str,
        session: &str,
        turn: Option<&str>,
        log: &mut RecallLog,
    ) -> Result<(), StateError> {
        let mut writer = self.try_writer()?;
        writer.busy_timeout(std::time::Duration::from_millis(5))?;
        let outcome = (|| {
            let transaction = writer.transaction()?;
            let mut injected = Vec::new();
            for id in &log.injected {
                if transaction.execute(
                    "INSERT OR IGNORE INTO recall_injected (agent, session, fact) VALUES (?1, ?2, ?3)",
                    params![agent, session, id.to_string()],
                )? == 1
                {
                    injected.push(*id);
                }
            }
            let candidates: Vec<_> =
                log.candidates.iter().map(|score| (score.id.to_string(), score.probability)).collect();
            transaction.execute("INSERT INTO recall_log (at, gate, queries, source, candidates, injected, timings, models, agent, session, turn) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![
            at.as_millis(), log.gate, i64::try_from(log.queries).unwrap_or(i64::MAX), log.source,
            serde_json::json!(candidates).to_string(), serde_json::json!(injected).to_string(),
            serde_json::json!(log.timings).to_string(), serde_json::json!(log.models).to_string(), agent, session, turn
        ])?;
            transaction.commit()?;
            Ok::<_, StateError>(injected)
        })();
        let reset = writer.busy_timeout(std::time::Duration::from_secs(5));
        log.injected = outcome?;
        reset?;
        Ok(())
    }
}
