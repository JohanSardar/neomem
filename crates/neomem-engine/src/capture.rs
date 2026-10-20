use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use neomem_core::{AgentAdapter, CaptureSignal, LoggedIntake};
use neomem_proto::{FailureKind, Response};

use crate::Engine;
use crate::reply::failed;

#[derive(Debug, Clone)]
pub struct CaptureSource {
    pub root: PathBuf,
    pub adapter: Arc<dyn AgentAdapter>,
}

#[derive(Debug)]
pub(crate) struct CaptureRuntime {
    sources: Vec<CaptureSource>,
    every: Duration,
    swept: Option<Instant>,
}

impl Default for CaptureRuntime {
    fn default() -> Self {
        Self { sources: Vec::new(), every: Duration::from_secs(60), swept: None }
    }
}

impl Engine {
    pub fn configure_capture(&self, sources: Vec<CaptureSource>, every: Duration) {
        if let Ok(mut capture) = self.capture.lock() {
            *capture = CaptureRuntime { sources, every, swept: None };
        }
    }

    pub(crate) fn capture_signal(&self, signal: &CaptureSignal) -> Response {
        let sources = self.capture_sources();
        for source in sources.iter().filter(|source| source.adapter.agent() == signal.agent) {
            let paths = if let Some(path) = &signal.transcript_path {
                vec![path.clone()]
            } else {
                transcripts_below(&source.root)
            };
            for path in paths {
                if let Err(error) = self.capture_transcript(source, &path, Some(&signal.session)) {
                    return failed(FailureKind::Internal, error);
                }
            }
        }
        if signal.event.immediate()
            && let Err(error) = self.state.hasten_batch(
                &neomem_state::BatchKey::for_session(signal.agent.source(), &signal.session),
                self.clock.now(),
            )
        {
            return failed(FailureKind::Internal, error.to_string());
        }
        Response::Captured
    }

    pub(crate) fn sweep_transcripts(&self) {
        let due = self.capture.lock().is_ok_and(|mut capture| {
            if capture.swept.is_some_and(|last| last.elapsed() < capture.every) {
                return false;
            }
            capture.swept = Some(Instant::now());
            true
        });
        if !due {
            return;
        }
        for source in self.capture_sources() {
            for path in transcripts_below(&source.root) {
                if let Err(error) = self.capture_transcript(&source, &path, None) {
                    tracing::warn!("transcript capture failed: {error}");
                }
            }
        }
    }

    pub(crate) fn transcript_changed(&self, path: &Path) {
        for source in self.capture_sources().iter().filter(|source| path.starts_with(&source.root)) {
            if let Err(error) = self.capture_transcript(source, path, None) {
                tracing::warn!("transcript capture failed: {error}");
            }
        }
    }

    fn capture_sources(&self) -> Vec<CaptureSource> {
        self.capture.lock().map(|capture| capture.sources.clone()).unwrap_or_default()
    }

    fn capture_transcript(
        &self,
        source: &CaptureSource,
        path: &Path,
        session: Option<&str>,
    ) -> Result<(), String> {
        if !path.is_file() {
            return Ok(());
        }
        let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
        let _writer = self.writer.lock().map_err(|error| error.to_string())?;
        let path_text = path.to_string_lossy();
        let agent = source.adapter.agent().source();
        let position =
            self.state.transcript_position(agent, &path_text).map_err(|error| error.to_string())?;
        let captured = source.adapter.read_new(&path, position).map_err(|error| error.to_string())?;
        if session.is_some_and(|session| !session.is_empty() && session != captured.session) {
            return Ok(());
        }
        let now = self.clock.now();
        let records = captured
            .records
            .into_iter()
            .map(|record| LoggedIntake { id: self.ids.intake(now), recorded_at: now, record })
            .collect::<Vec<_>>();
        self.state
            .log_transcript(
                &neomem_state::TranscriptPosition {
                    agent,
                    path: &path_text,
                    session: &captured.session,
                    position: captured.position,
                },
                &records,
                now.plus_millis(crate::worker_config::millis(self.worker.session_silence)),
            )
            .map_err(|error| error.to_string())
    }
}

fn transcripts_below(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return found };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            found.extend(transcripts_below(&path));
        } else if path.extension().is_some_and(|extension| extension == "jsonl") {
            found.push(path);
        }
    }
    found.sort();
    found
}
