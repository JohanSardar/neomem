use neomem_core::{EpochMs, IntakeId, IntakeRecord, MemoryId, ModelRole, Source, Tag};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum Request {
    Status,
    Capture(neomem_core::CaptureSignal),
    Shutdown,
    Add(IntakeRecord),
    Raw(RawQuery),
    Correct {
        intake: IntakeId,
        reason: String,
    },
    Rebuild,
    Reextract {
        #[serde(default)]
        source: Option<Source>,
        #[serde(default)]
        from: Option<EpochMs>,
    },
    CheckDrift,
    Dream,
    Search(SearchQuery),
    SetClock {
        now: EpochMs,
    },
    Resolve {
        name: String,
    },
    Entity {
        name: String,
    },
    Show {
        id: MemoryId,
    },
    History {
        id: neomem_core::FactId,
    },
    Useful {
        id: neomem_core::FactId,
    },
    Models,
    ModelsUse {
        role: ModelRole,
        model: String,
    },
    ModelsTest {
        role: ModelRole,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<Source>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        structured: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inputs: Vec<String>,
    },
    Login {
        provider: String,
    },
    LoginFinish {
        provider: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub k: Option<usize>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub from: Option<EpochMs>,
    #[serde(default)]
    pub to: Option<EpochMs>,
    #[serde(default)]
    pub as_of: Option<EpochMs>,
    #[serde(default)]
    pub include_invalid: bool,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub trace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawQuery {
    pub text: String,
    #[serde(default)]
    pub from: Option<EpochMs>,
    #[serde(default)]
    pub to: Option<EpochMs>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default = "default_raw_limit")]
    pub limit: usize,
}

fn default_raw_limit() -> usize {
    20
}

impl Request {
    pub fn waits_in_spool_when_the_daemon_is_down(&self) -> bool {
        matches!(self, Self::Add(_) | Self::Capture(_))
    }
}
