use std::time::Duration;

pub const DEFAULT_OWNER: &str = "the user";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub session_silence: Duration,
    pub batch_every: Duration,
    pub batch_size: usize,
    pub lease: Duration,
    pub owner: String,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            session_silence: Duration::from_mins(5),
            batch_every: Duration::from_mins(30),
            batch_size: 20,
            lease: Duration::from_mins(10),
            owner: DEFAULT_OWNER.to_owned(),
        }
    }
}

pub(crate) fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
