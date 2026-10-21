use std::fmt::Debug;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::IntakeRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureAgent {
    Cc,
    Codex,
    Pi,
}

impl CaptureAgent {
    pub fn source(self) -> &'static str {
        match self {
            Self::Cc => "claude-code",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureEvent {
    Prompt,
    Tool,
    Stop,
    SubagentStop,
    Compact,
    End,
}

impl CaptureEvent {
    pub fn immediate(self) -> bool {
        matches!(self, Self::Compact | Self::End)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSignal {
    pub agent: CaptureAgent,
    pub session: String,
    pub transcript_path: Option<PathBuf>,
    pub event: CaptureEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub session: String,
    pub position: u64,
    pub records: Vec<IntakeRecord>,
}

#[derive(Debug, thiserror::Error)]
#[error("reading transcript failed: {0}")]
pub struct CaptureError(pub String);

pub trait AgentAdapter: Send + Sync + Debug {
    fn agent(&self) -> CaptureAgent;
    fn read_new(&self, file: &Path, position: u64) -> Result<Captured, CaptureError>;
    fn slice(&self, session: &Path) -> Result<Vec<IntakeRecord>, CaptureError> {
        self.read_new(session, 0).map(|captured| captured.records)
    }
}
