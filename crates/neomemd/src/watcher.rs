use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use neomem_engine::{Engine, Notice};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

const SETTLE: Duration = Duration::from_millis(100);

pub fn watch(engine: Arc<Engine>, directories: &[PathBuf]) -> anyhow::Result<RecommendedWatcher> {
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        if let Ok(event) = event {
            let _ = sender.send(event);
        }
    })
    .context("starting the file watcher")?;
    for directory in directories.iter().filter(|directory| directory.exists()) {
        watcher
            .watch(directory, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", directory.display()))?;
    }
    thread::Builder::new().name("neomemd-watcher".to_owned()).spawn(move || deliver(&engine, &receiver))?;
    Ok(watcher)
}

fn deliver(engine: &Engine, events: &Receiver<Event>) {
    while let Ok(first) = events.recv() {
        let mut changed = BTreeSet::new();
        collect(first, &mut changed);
        let settled_at = Instant::now() + SETTLE;
        loop {
            match events.recv_timeout(settled_at.saturating_duration_since(Instant::now())) {
                Ok(event) => collect(event, &mut changed),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        for path in changed {
            engine.notice(Notice::FileChanged(path));
        }
    }
}

fn collect(event: Event, changed: &mut BTreeSet<PathBuf>) {
    if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
        return;
    }
    for path in event.paths {
        if path.is_dir() {
            watched_files_below(&path, changed);
        } else if is_watched_file(&path) {
            changed.insert(path);
        }
    }
}

fn watched_files_below(directory: &Path, changed: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if path.is_dir() {
            watched_files_below(&path, changed);
        } else if is_watched_file(&path) {
            changed.insert(path);
        }
    }
}

fn is_watched_file(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "md" || extension == "jsonl")
}
