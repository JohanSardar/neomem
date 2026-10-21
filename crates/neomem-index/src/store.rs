use std::path::{Path, PathBuf};
use std::time::Duration;

use half::f16;
use neomem_core::{Entity, Fact, FactId, LinkKind, Memory, MemoryId, ModelId, Vector};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::IndexError;

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r"
DROP TABLE IF EXISTS active_model;
DROP TABLE IF EXISTS memories;
DROP TABLE IF EXISTS vectors;
DROP TABLE IF EXISTS links;
DROP TABLE IF EXISTS names;
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    payload TEXT NOT NULL,
    path TEXT NOT NULL,
    latest_id TEXT,
    recorded_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE TABLE vectors (
    memory_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (memory_id, model_id)
) STRICT;
CREATE TABLE links (
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    type TEXT NOT NULL,
    valid_at INTEGER,
    invalid_at INTEGER,
    recorded_at INTEGER NOT NULL
) STRICT;
CREATE INDEX links_by_from ON links (from_id);
CREATE INDEX links_by_to ON links (to_id);
CREATE TABLE names (
    name TEXT NOT NULL,
    entity_id TEXT NOT NULL
) STRICT;
CREATE INDEX names_by_name ON names (name);
";

#[derive(Debug, Clone)]
pub struct Change {
    pub memory: Memory,
    pub path: PathBuf,
    pub vector: Option<Vector>,
}

#[derive(Debug)]
pub(crate) struct IndexDb {
    connection: Connection,
}

pub(crate) struct LoadedRow {
    pub(crate) memory: Memory,
    pub(crate) vector: Option<Vector>,
    pub(crate) path: PathBuf,
}

impl IndexDb {
    pub(crate) fn open(path: &Path) -> Result<(Self, bool), IndexError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let fresh = version != SCHEMA_VERSION;
        if fresh {
            connection.execute_batch(SCHEMA)?;
            connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS active_model (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), model TEXT NOT NULL, dims INTEGER NOT NULL, cleanup INTEGER NOT NULL DEFAULT 0) STRICT;")?;
        Ok((Self { connection }, fresh))
    }

    pub(crate) fn load(&self, model: Option<&ModelId>) -> Result<Vec<LoadedRow>, IndexError> {
        self.load_rows(model, false, -1)
    }

    pub(crate) fn missing(&self, model: &ModelId, limit: usize) -> Result<Vec<LoadedRow>, IndexError> {
        self.load_rows(Some(model), true, i64::try_from(limit).unwrap_or(i64::MAX))
    }

    fn load_rows(
        &self,
        model: Option<&ModelId>,
        missing: bool,
        limit: i64,
    ) -> Result<Vec<LoadedRow>, IndexError> {
        let mut statement = self.connection.prepare(
            "SELECT m.kind, m.payload, v.data, m.path FROM memories m
             LEFT JOIN vectors v ON v.memory_id = m.id AND v.model_id = ?1 WHERE (?2 = 0 OR v.data IS NULL) LIMIT ?3",
        )?;
        let model_name = model.map(ModelId::as_str);
        let rows = statement.query_map(params![model_name, missing, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut loaded = Vec::new();
        for row in rows {
            let (kind, payload, data, path) = row?;
            let memory = decode(&kind, &payload)?;
            loaded.push(LoadedRow {
                memory,
                vector: data.as_deref().map(vector_from_bytes),
                path: PathBuf::from(path),
            });
        }
        Ok(loaded)
    }

    pub(crate) fn write(
        &mut self,
        changes: &[Change],
        model: Option<&ModelId>,
        clear_first: bool,
    ) -> Result<(), IndexError> {
        let transaction = self.connection.transaction()?;
        if clear_first {
            transaction.execute_batch(
                "DELETE FROM memories; DELETE FROM vectors; DELETE FROM links; DELETE FROM names;",
            )?;
        }
        for change in changes {
            transaction.execute(
                "DELETE FROM vectors WHERE memory_id = ?1 AND (?2 IS NULL OR model_id != ?2)",
                params![change.memory.id().to_string(), model.map(ModelId::as_str)],
            )?;
            write_change(&transaction, change, model)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn active_model(&self) -> Result<Option<(ModelId, usize)>, IndexError> {
        let active: Option<(String, i64)> = self
            .connection
            .query_row("SELECT model, dims FROM active_model WHERE singleton = 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        active
            .map(|(model, dims)| {
                let dims = usize::try_from(dims)
                    .map_err(|_| IndexError::Corrupt("invalid active embedding dimensions".to_owned()))?;
                Ok((ModelId::new(model), dims))
            })
            .transpose()
    }

    pub(crate) fn activate(&mut self, model: &ModelId, dims: usize) -> Result<(), IndexError> {
        let dims = i64::try_from(dims)
            .map_err(|_| IndexError::Corrupt("invalid embedding dimensions".to_owned()))?;
        let transaction = self.connection.transaction()?;
        transaction.execute("INSERT INTO active_model (singleton, model, dims, cleanup) VALUES (1, ?1, ?2, 1) ON CONFLICT(singleton) DO UPDATE SET model = ?1, dims = ?2, cleanup = 1", params![model.as_str(), dims])?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn finish_activation(&mut self) -> Result<(), IndexError> {
        let pending: bool =
            self.connection
                .query_row("SELECT coalesce(max(cleanup), 0) FROM active_model", [], |row| row.get(0))?;
        if pending {
            let transaction = self.connection.transaction()?;
            transaction.execute(
                "DELETE FROM vectors WHERE model_id != (SELECT model FROM active_model WHERE singleton = 1)",
                [],
            )?;
            transaction.execute("UPDATE active_model SET cleanup = 0 WHERE singleton = 1", [])?;
            transaction.commit()?;
        }
        Ok(())
    }

    pub(crate) fn stage_vectors(&mut self, model: &ModelId, changes: &[Change]) -> Result<(), IndexError> {
        let transaction = self.connection.transaction()?;
        for change in changes {
            if let Some(vector) = &change.vector {
                transaction.execute("INSERT INTO vectors (memory_id, model_id, data) VALUES (?1, ?2, ?3) ON CONFLICT(memory_id, model_id) DO UPDATE SET data = ?3", params![change.memory.id().to_string(), model.as_str(), vector_to_bytes(vector)])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn record_latest(&mut self, heads: &[(FactId, FactId)]) -> Result<(), IndexError> {
        let transaction = self.connection.transaction()?;
        {
            let mut update =
                transaction.prepare_cached("UPDATE memories SET latest_id = ?2 WHERE id = ?1")?;
            for (fact, head) in heads {
                update.execute(params![fact.to_string(), head.to_string()])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

fn write_change(
    transaction: &Transaction<'_>,
    change: &Change,
    model: Option<&ModelId>,
) -> Result<(), IndexError> {
    let id = change.memory.id().to_string();
    let (kind, payload, recorded_at, updated_at) = match &change.memory {
        Memory::Fact(fact) => ("fact", encode(fact)?, fact.recorded_at(), fact.updated_at()),
        Memory::Entity(entity) => ("entity", encode(entity)?, entity.recorded_at(), entity.updated_at()),
    };
    transaction.execute(
        "INSERT INTO memories (id, kind, content, payload, path, latest_id, recorded_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?1, ?6, ?7)
         ON CONFLICT (id) DO UPDATE SET content = ?3, payload = ?4, path = ?5, updated_at = ?7",
        params![
            id,
            kind,
            change.memory.embedding_text(),
            payload,
            change.path.to_string_lossy(),
            recorded_at.as_millis(),
            updated_at.as_millis()
        ],
    )?;
    transaction.execute("DELETE FROM links WHERE from_id = ?1", [&id])?;
    for (to, link, valid_at, invalid_at) in outgoing_links(&change.memory) {
        transaction.execute(
            "INSERT INTO links (from_id, to_id, type, valid_at, invalid_at, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, to.to_string(), link.name(), valid_at, invalid_at, updated_at.as_millis()],
        )?;
    }
    if let Memory::Entity(entity) = &change.memory {
        transaction.execute("DELETE FROM names WHERE entity_id = ?1", [&id])?;
        for name in entity.names() {
            transaction.execute(
                "INSERT INTO names (name, entity_id) VALUES (?1, ?2)",
                params![name.trim().to_lowercase(), id],
            )?;
        }
    }
    if let (Some(vector), Some(model)) = (&change.vector, model) {
        transaction.execute(
            "INSERT INTO vectors (memory_id, model_id, data) VALUES (?1, ?2, ?3)
             ON CONFLICT (memory_id, model_id) DO UPDATE SET data = ?3",
            params![id, model.as_str(), vector_to_bytes(vector)],
        )?;
    }
    Ok(())
}

type LinkRow = (MemoryId, LinkKind, Option<i64>, Option<i64>);

fn outgoing_links(memory: &Memory) -> Vec<LinkRow> {
    match memory {
        Memory::Fact(fact) => {
            let structural =
                fact.links().iter().map(|link| (MemoryId::Fact(link.to), link.kind.into(), None, None));
            let about =
                fact.about().iter().map(|entity| (MemoryId::Entity(*entity), LinkKind::About, None, None));
            structural.chain(about).collect()
        }
        Memory::Entity(entity) => {
            let links =
                entity.links().iter().map(|link| (MemoryId::Entity(link.to), link.kind.into(), None, None));
            let relations = entity.relations().iter().map(|relation| {
                (
                    MemoryId::Entity(relation.to),
                    LinkKind::Relation(relation.label.clone()),
                    Some(relation.valid_at.as_millis()),
                    relation.invalid_at.map(neomem_core::EpochMs::as_millis),
                )
            });
            links.chain(relations).collect()
        }
    }
}

fn encode(value: &impl serde::Serialize) -> Result<String, IndexError> {
    serde_json::to_string(value).map_err(|error| IndexError::Corrupt(error.to_string()))
}

fn decode(kind: &str, payload: &str) -> Result<Memory, IndexError> {
    let corrupt = |error: serde_json::Error| IndexError::Corrupt(error.to_string());
    match kind {
        "fact" => serde_json::from_str::<Fact>(payload).map(Memory::Fact).map_err(corrupt),
        "entity" => serde_json::from_str::<Entity>(payload).map(Memory::Entity).map_err(corrupt),
        other => Err(IndexError::Corrupt(format!("unknown memory kind {other}"))),
    }
}

fn vector_to_bytes(vector: &Vector) -> Vec<u8> {
    vector.as_slice().iter().flat_map(|value| f16::from_f32(*value).to_le_bytes()).collect()
}

fn vector_from_bytes(bytes: &[u8]) -> Vector {
    Vector::normalized(
        bytes.as_chunks::<2>().0.iter().map(|pair| f16::from_le_bytes(*pair).to_f32()).collect(),
    )
}
