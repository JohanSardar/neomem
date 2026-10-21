use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use neomem_core::{Clock, EpochMs, ModelError};

const BACKGROUND_GRACE: Duration = Duration::from_mins(2);

#[derive(Debug)]
pub(crate) struct LocalSlot<T> {
    state: Mutex<State<T>>,
    last_read: Arc<AtomicI64>,
    last_used: AtomicI64,
    used: AtomicBool,
}

#[derive(Debug)]
struct State<T> {
    loaded: Option<T>,
}

impl<T> LocalSlot<T> {
    pub(crate) fn new(last_read: Arc<AtomicI64>) -> Self {
        Self {
            state: Mutex::new(State { loaded: None }),
            last_read,
            last_used: AtomicI64::new(i64::MIN),
            used: AtomicBool::new(false),
        }
    }

    pub(crate) fn run<R>(
        &self,
        load: impl FnOnce() -> Result<T, ModelError>,
        work: impl FnOnce(&T) -> Result<R, ModelError>,
    ) -> Result<R, ModelError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.loaded.is_none() {
            state.loaded = Some(load()?);
        }
        self.used.store(true, Ordering::SeqCst);
        match &state.loaded {
            Some(loaded) => work(loaded),
            None => Err(ModelError::Unavailable("local model did not load".to_owned())),
        }
    }

    pub(crate) fn maintain(&self, now: EpochMs, timeout: Duration) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.used.swap(false, Ordering::SeqCst) {
            self.last_used.store(now.as_millis(), Ordering::SeqCst);
        }
        if idle_for(now, &self.last_read, timeout)
            && idle_for(now, &self.last_used, BACKGROUND_GRACE.min(timeout))
        {
            state.loaded = None;
        }
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).loaded.is_some()
    }
}

fn idle_for(now: EpochMs, last: &AtomicI64, wait: Duration) -> bool {
    now.millis_since(EpochMs::from_millis(last.load(Ordering::SeqCst)))
        >= i64::try_from(wait.as_millis()).unwrap_or(i64::MAX)
}

#[derive(Debug)]
pub(crate) struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> EpochMs {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        EpochMs::from_millis(i64::try_from(millis).unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use super::*;

    #[derive(Debug, Default)]
    struct TestClock(AtomicI64);

    impl Clock for TestClock {
        fn now(&self) -> EpochMs {
            EpochMs::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    #[derive(Debug)]
    struct Resource(Arc<AtomicUsize>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn idle_resources_are_released_and_reloaded_once() {
        let clock = Arc::new(TestClock::default());
        let last_read = Arc::new(AtomicI64::new(i64::MIN));
        let slot = LocalSlot::new(Arc::clone(&last_read));
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        let use_model = || {
            last_read.store(clock.now().as_millis(), Ordering::SeqCst);
            slot.run(
                || {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(Resource(Arc::clone(&drops)))
                },
                |_| Ok(()),
            )
            .unwrap();
        };
        assert!(!slot.is_loaded());
        use_model();
        slot.maintain(EpochMs::from_millis(899_999), Duration::from_mins(15));
        assert!(slot.is_loaded());
        clock.0.store(900_000, Ordering::SeqCst);
        slot.maintain(clock.now(), Duration::from_mins(15));
        assert!(slot.is_loaded());
        clock.0.store(1_100_000, Ordering::SeqCst);
        slot.maintain(clock.now(), Duration::from_mins(15));
        assert!(!slot.is_loaded());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        use_model();
        use_model();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;

    #[test]
    fn background_use_keeps_a_model_only_for_a_short_grace_not_the_read_idle_window() {
        let last_read = Arc::new(AtomicI64::new(0));
        let slot = LocalSlot::new(last_read);
        let loads = std::sync::atomic::AtomicUsize::new(0);
        let work = || {
            slot.run(
                || {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(7)
                },
                |value| Ok(*value),
            )
            .unwrap()
        };
        for tick in 0..10 {
            work();
            slot.maintain(EpochMs::from_millis(900_000 + tick * 1_000), Duration::from_mins(15));
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        slot.maintain(EpochMs::from_millis(909_000 + 120_000), Duration::from_mins(15));
        assert!(!slot.is_loaded());
    }
}
