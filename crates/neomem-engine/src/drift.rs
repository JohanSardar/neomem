use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use neomem_core::{Memory, MemoryId};
use neomem_index::Snapshot;
use neomem_proto::{DriftReport, FailureKind, Response};
use neomem_state::RunRecord;

use crate::Engine;
use crate::reply::failed;

impl Engine {
    pub(crate) fn drift(&self) -> Response {
        match self.check_drift() {
            Ok(report) => Response::DriftChecked(report),
            Err(error) => failed(FailureKind::Internal, format!("the drift check failed: {error}")),
        }
    }

    pub(crate) fn check_drift(&self) -> Result<DriftReport, crate::indexing::IndexingError> {
        let started = Instant::now();
        let started_at = self.clock.now();
        let snapshot = self.index.snapshot();
        let mut on_disk = HashSet::new();
        let mut differing: Vec<(Memory, PathBuf)> = Vec::new();
        let (mut added, mut changed, mut unreadable) = (0, 0, 0);
        for scanned in self.vault.scan() {
            let Ok((path, memory)) = scanned else {
                unreadable += 1;
                continue;
            };
            on_disk.insert(memory.id());
            match indexed_version_matches(&snapshot, &memory) {
                None => added += 1,
                Some(false) => changed += 1,
                Some(true) => continue,
            }
            differing.push((memory, path));
        }
        let missing_files = indexed_ids(&snapshot).filter(|id| !on_disk.contains(id)).count();
        if missing_files > 0 {
            self.rebuild_index()?;
        } else if !differing.is_empty() {
            self.index_memories(differing)?;
        }
        let report = DriftReport {
            differences: added + changed + missing_files,
            added,
            changed,
            missing_files,
            unreadable,
            took_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
        };
        self.log_drift_run(started_at, &report);
        Ok(report)
    }

    fn log_drift_run(&self, started_at: neomem_core::EpochMs, report: &DriftReport) {
        let run = RunRecord {
            kind: "drift",
            started_at,
            finished_at: self.clock.now(),
            model: None,
            changes: serde_json::json!({
                "differences": report.differences,
                "added": report.added,
                "changed": report.changed,
                "missing_files": report.missing_files,
                "unreadable": report.unreadable,
            }),
        };
        if let Err(error) = self.state.log_run(&run) {
            tracing::error!("recording the drift run failed: {error}");
        }
    }
}

fn indexed_version_matches(snapshot: &Snapshot, memory: &Memory) -> Option<bool> {
    match memory {
        Memory::Fact(fact) => snapshot.fact(fact.id()).map(|indexed| **indexed == *fact),
        Memory::Entity(entity) => snapshot.entity(entity.id()).map(|indexed| **indexed == *entity),
    }
}

fn indexed_ids(snapshot: &Snapshot) -> impl Iterator<Item = MemoryId> + '_ {
    let facts = snapshot.facts().map(|fact| MemoryId::Fact(fact.id()));
    facts.chain(snapshot.entities().map(|entity| MemoryId::Entity(entity.id())))
}
