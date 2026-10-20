use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use neomem_core::{Clock, EpochMs};

#[derive(Debug)]
pub struct DaemonClock {
    offset_ms: AtomicI64,
    settable: bool,
}

impl DaemonClock {
    pub fn new(settable: bool) -> Self {
        Self { offset_ms: AtomicI64::new(0), settable }
    }

    pub fn set(&self, now: EpochMs) -> bool {
        if self.settable {
            self.offset_ms.store(now.millis_since(system_now()), Ordering::Relaxed);
        }
        self.settable
    }
}

impl Clock for DaemonClock {
    fn now(&self) -> EpochMs {
        system_now().plus_millis(self.offset_ms.load(Ordering::Relaxed))
    }
}

fn system_now() -> EpochMs {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    EpochMs::from_millis(i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_settable_clock_moves() {
        let fixed = DaemonClock::new(false);
        assert!(!fixed.set(EpochMs::from_millis(0)));
        assert!(fixed.now().as_millis() > 1_700_000_000_000);
        let settable = DaemonClock::new(true);
        assert!(settable.set(EpochMs::from_millis(5_000_000_000_000)));
        assert!(settable.now().as_millis() >= 5_000_000_000_000);
    }
}
