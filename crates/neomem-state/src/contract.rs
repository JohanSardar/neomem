use neomem_core::{ExternalLink, Tag};
use rusqlite::params;

use crate::{StateDb, StateError};

impl StateDb {
    pub fn tags(&self) -> Result<Vec<Tag>, StateError> {
        self.read(|connection| {
            let mut statement = connection.prepare("SELECT name FROM tags ORDER BY name")?;
            let names = statement.query_map([], |row| row.get::<_, String>(0))?;
            names
                .map(|name| Tag::new(&name?).map_err(|error| StateError::Corrupt(error.to_string())))
                .collect()
        })
    }

    pub fn register_tag(&self, tag: &Tag) -> Result<(), StateError> {
        self.writer()?.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag.as_str()])?;
        Ok(())
    }

    pub fn register_external_link(&self, link: &ExternalLink) -> Result<(), StateError> {
        self.writer()?.execute(
            "INSERT OR IGNORE INTO external_links (external, target, kind, recorded_at) VALUES (?1, ?2, ?3, ?4)",
            params![link.external.to_string(), link.to.to_string(), link.kind.name(), link.recorded_at.as_millis()],
        )?;
        Ok(())
    }

    pub fn external_links(&self) -> Result<Vec<ExternalLink>, StateError> {
        self.read(|connection| {
            let mut statement = connection.prepare("SELECT external, target, kind, recorded_at FROM external_links ORDER BY external, target, kind")?;
            let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)))?;
            rows.map(|row| {
                let (external, target, kind, at) = row?;
                Ok(ExternalLink {
                    external: external.parse().map_err(|error: neomem_core::InvalidExternalRef| StateError::Corrupt(error.to_string()))?,
                    to: target.parse().map_err(|error: neomem_core::InvalidId| StateError::Corrupt(error.to_string()))?,
                    kind: kind.parse().map_err(StateError::Corrupt)?,
                    recorded_at: neomem_core::EpochMs::from_millis(at),
                })
            }).collect()
        })
    }
}
