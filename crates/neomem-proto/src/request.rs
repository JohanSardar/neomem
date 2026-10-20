use neomem_core::{EpochMs, IntakeId, IntakeRecord, MemoryId, ModelRole, Source};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum Request {
    Status,
    Shutdown,
    Add(IntakeRecord),
    Raw(RawQuery),
    Correct {
        intake: IntakeId,
        reason: String,
    },
    Rebuild,
    Resolve {
        name: String,
    },
    Entity {
        name: String,
    },
    Show {
        id: MemoryId,
    },
    Models,
    ModelsTest {
        role: ModelRole,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<Source>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        structured: bool,
    },
    Login {
        provider: String,
    },
    LoginFinish {
        provider: String,
    },
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
        matches!(self, Self::Add(_))
    }
}
