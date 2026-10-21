use rusqlite::Connection;

use crate::StateError;

const MIGRATIONS: &[&str] = &[
    r"
CREATE TABLE intake (
    id TEXT NOT NULL UNIQUE,
    recorded_at INTEGER NOT NULL,
    source TEXT NOT NULL,
    occurred_start INTEGER NOT NULL,
    occurred_end INTEGER NOT NULL,
    tags TEXT NOT NULL,
    content TEXT NOT NULL,
    entity_hints TEXT NOT NULL,
    source_ref TEXT,
    session TEXT
) STRICT;
CREATE INDEX intake_by_occurred ON intake (occurred_start);
CREATE INDEX intake_by_session ON intake (source, session, recorded_at);
CREATE VIRTUAL TABLE intake_text USING fts5 (content, content = 'intake', content_rowid = 'rowid');
CREATE TRIGGER intake_text_follows_inserts AFTER INSERT ON intake BEGIN
    INSERT INTO intake_text (rowid, content) VALUES (new.rowid, new.content);
END;
CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    batch_key TEXT NOT NULL,
    intake_id TEXT,
    state TEXT NOT NULL,
    not_before INTEGER NOT NULL,
    lease_until INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    finished_at INTEGER
) STRICT;
CREATE INDEX jobs_by_state ON jobs (kind, state, batch_key, not_before);
",
    r"
CREATE TABLE errors (
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL
) STRICT;
CREATE INDEX errors_by_kind ON errors (kind, at);
",
    r"
CREATE TABLE corrections (
    intake_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    recorded_at INTEGER NOT NULL
) STRICT;
CREATE INDEX corrections_by_intake ON corrections (intake_id);
",
    r"
CREATE TABLE runs (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    model TEXT,
    changes TEXT NOT NULL
) STRICT;
CREATE INDEX runs_by_kind ON runs (kind, started_at);
",
    r"
CREATE TABLE transcript_positions (
    agent TEXT NOT NULL,
    path TEXT NOT NULL,
    session TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (agent, path)
) STRICT;
",
    r"
CREATE TABLE transcript_entries (
    agent TEXT NOT NULL,
    path TEXT NOT NULL,
    turn TEXT NOT NULL,
    PRIMARY KEY (agent, path, turn)
) STRICT;
",
    r"
CREATE TABLE recall_log (
    id INTEGER PRIMARY KEY,
    agent TEXT NOT NULL,
    session TEXT NOT NULL,
    turn TEXT,
    at INTEGER NOT NULL,
    gate REAL,
    queries INTEGER NOT NULL,
    source TEXT NOT NULL,
    candidates TEXT NOT NULL,
    injected TEXT NOT NULL,
    timings TEXT NOT NULL,
    models TEXT NOT NULL
) STRICT;
CREATE INDEX recall_by_turn ON recall_log (agent, session, turn, id);
CREATE TABLE recall_search_labels (
    recall_id INTEGER NOT NULL UNIQUE REFERENCES recall_log(id),
    searched_at INTEGER NOT NULL
) STRICT;
CREATE TABLE recall_injected (
    agent TEXT NOT NULL,
    session TEXT NOT NULL,
    fact TEXT NOT NULL,
    PRIMARY KEY (agent, session, fact)
) STRICT;
",
];

pub fn migrate(connection: &mut Connection) -> Result<(), StateError> {
    let applied: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (version, migration) in (1_i64..).zip(MIGRATIONS.iter()).filter(|(version, _)| *version > applied) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}
