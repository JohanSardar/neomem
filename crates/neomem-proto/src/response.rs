use neomem_core::{
    Direction, Entity, EntityId, EpochMs, Fact, IntakeId, IntakeRecord, Label, LinkKind, MemoryId, ModelRole,
    RoleAssignment, Source, Span, Tag,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", content = "body", rename_all = "snake_case")]
pub enum Response {
    Status(StatusReport),
    Added(AddReceipt),
    RawHits(Vec<RawHit>),
    Corrected(CorrectionReceipt),
    Rebuilt(RebuildReport),
    Resolved(Vec<ResolvedEntity>),
    Entity(Box<EntityView>),
    Shown(Box<ShownMemory>),
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
    #[serde(default)]
    pub index: Option<IndexReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReport {
    pub facts: usize,
    pub entities: usize,
    pub links: usize,
    pub names: usize,
    pub vectors: usize,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildReport {
    pub index: IndexReport,
    pub unreadable_files: usize,
    pub took_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddReceipt {
    pub id: IntakeId,
    pub recorded_at: EpochMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionReceipt {
    pub intake: IntakeId,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEntity {
    pub id: EntityId,
    pub name: String,
    pub kind: Label,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityView {
    pub entity: Entity,
    pub facts: Vec<Fact>,
    pub linked: Vec<LinkedEntity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedEntity {
    pub kind: LinkKind,
    pub direction: Direction,
    pub entity: Entity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownMemory {
    pub id: MemoryId,
    pub path: String,
    pub text: String,
    pub sources: Vec<SourceView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceView {
    pub id: neomem_core::IntakeId,
    pub record: Option<IntakeRecord>,
    pub corrected: bool,
}
