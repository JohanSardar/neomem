use std::fmt;

use neomem_core::{EpochMs, LoggedIntake};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BatchKey(String);

impl BatchKey {
    pub fn for_record(logged: &LoggedIntake) -> Self {
        let record = &logged.record;
        match &record.session {
            Some(session) => Self(format!("{}/{session}", record.source)),
            None => Self(record.source.to_string()),
        }
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
