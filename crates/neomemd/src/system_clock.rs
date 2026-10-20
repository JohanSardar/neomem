use std::time::{SystemTime, UNIX_EPOCH};

use neomem_core::{Clock, EpochMs};

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> EpochMs {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        EpochMs::from_millis(i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX))
    }
}
