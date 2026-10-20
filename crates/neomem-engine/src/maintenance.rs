use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::Engine;

const TICK: Duration = Duration::from_millis(250);
const SPOOL_EVERY_TICKS: u64 = 8;

impl Engine {
    pub fn run(self: &Arc<Self>) -> std::io::Result<thread::JoinHandle<()>> {
        let engine = Arc::clone(self);
        thread::Builder::new().name("neomemd-maintenance".to_owned()).spawn(move || {
            let mut ticks: u64 = 0;
            loop {
                if ticks % SPOOL_EVERY_TICKS == 0 {
                    engine.drain_spool();
                }
                engine.maintain();
                ticks = ticks.wrapping_add(1);
                thread::sleep(TICK);
            }
        })
    }

    fn maintain(&self) {
        match self.vault.commit_pending(self.clock.now(), self.commit_policy) {
            Ok(Some(changes)) => tracing::info!("committed {changes} vault changes"),
            Ok(None) => {}
            Err(error) => tracing::error!("committing the vault failed: {error}"),
        }
    }

    pub(crate) fn commit_now(&self) {
        let everything = neomem_vault::CommitPolicy { every: Duration::ZERO, after_changes: 0 };
        if let Err(error) = self.vault.commit_pending(self.clock.now(), everything) {
            tracing::error!("committing the vault at shutdown failed: {error}");
        }
    }
}
