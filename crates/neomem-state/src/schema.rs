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
