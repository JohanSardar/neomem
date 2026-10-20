use neomem_core::{EpochMs, IntakeId, ModelRole, RoleAssignment, Source, Span, Tag};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", content = "body", rename_all = "snake_case")]
pub enum Response {
    Status(StatusReport),
    Added(AddReceipt),
    RawHits(Vec<RawHit>),
    ModelRoster(Vec<RoleAssignment>),
    ModelTested(ModelTestReport),
    ShuttingDown,
    VersionMismatch { daemon: u32, client: u32 },
    Failed(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    pub daemon_version: String,
    pub protocol: u32,
    pub home: String,
    pub pid: u32,
    pub uptime_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddReceipt {
    pub id: IntakeId,
    pub recorded_at: EpochMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawHit {
    pub id: IntakeId,
    pub source: Source,
    pub occurred: Span,
    pub recorded_at: EpochMs,
    pub tags: Vec<Tag>,
    pub content: String,
    pub corrected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelTestReport {
    pub role: ModelRole,
    pub model: String,
    pub latency_ms: f64,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    BadRequest,
    NotFound,
    Unavailable,
    Internal,
}

impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}
