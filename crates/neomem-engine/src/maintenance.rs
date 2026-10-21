use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::Engine;

const TICK: Duration = Duration::from_millis(250);
const SPOOL_EVERY_TICKS: u64 = 8;
const WORKER_IDLE: Duration = Duration::from_millis(200);

impl Engine {
    pub fn run(self: &Arc<Self>) -> std::io::Result<thread::JoinHandle<()>> {
        let worker = Arc::clone(self);
        thread::Builder::new().name("neomemd-worker".to_owned()).spawn(move || {
            loop {
                if !worker.work_once() {
                    thread::sleep(WORKER_IDLE);
                }
            }
        })?;
        let engine = Arc::clone(self);
        thread::Builder::new().name("neomemd-maintenance".to_owned()).spawn(move || {
            engine.heal_index();
            let mut ticks: u64 = 0;
            let mut next_nightly = engine.nightly_at.next_after(engine.clock.now());
            loop {
                if ticks.is_multiple_of(SPOOL_EVERY_TICKS) {
                    engine.drain_spool();
                }
                engine.sweep_transcripts();
                engine.maintain();
                if engine.clock.now() >= next_nightly {
                    engine.run_nightly();
                    next_nightly = engine.nightly_at.next_after(engine.clock.now());
                }
                ticks = ticks.wrapping_add(1);
                thread::sleep(TICK);
            }
        })
    }

    fn maintain(&self) {
        self.models.maintain(self.clock.now());
        self.advance_model_swap();
        match self.vault.commit_pending(self.clock.now(), self.commit_policy) {
            Ok(Some(changes)) => tracing::info!("committed {changes} vault changes"),
            Ok(None) => {}
            Err(error) => tracing::error!("committing the vault failed: {error}"),
        }
    }

    fn run_nightly(&self) {
        match self.check_drift() {
            Ok(report) => tracing::info!("nightly drift check: {} differences repaired", report.differences),
            Err(error) => tracing::error!("the nightly drift check failed: {error}"),
        }
    }

    pub(crate) fn commit_now(&self) {
        let everything = neomem_vault::CommitPolicy { every: Duration::ZERO, after_changes: 0 };
        if let Err(error) = self.vault.commit_pending(self.clock.now(), everything) {
            tracing::error!("committing the vault at shutdown failed: {error}");
        }
    }
}
