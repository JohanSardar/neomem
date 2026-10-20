use neomem_core::{EpochMs, IntakeId, ModelRole, RoleAssignment};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", content = "body", rename_all = "snake_case")]
pub enum Response {
    Status(StatusReport),
    Added(AddReceipt),
    ModelRoster(Vec<RoleAssignment>),
    ModelTested(ModelTestReport),
    LoginOpened { provider: String, url: String },
    LoggedIn { provider: String, account: String },
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelTestReport {
    pub role: ModelRole,
    pub model: String,
    pub latency_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_text_ms: Option<f64>,
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
