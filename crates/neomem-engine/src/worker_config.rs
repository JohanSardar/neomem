use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerConfig {
    pub session_silence: Duration,
    pub batch_every: Duration,
    pub batch_size: usize,
    pub lease: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            session_silence: Duration::from_mins(5),
            batch_every: Duration::from_mins(30),
            batch_size: 20,
            lease: Duration::from_mins(10),
        }
    }
}

pub(crate) fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
