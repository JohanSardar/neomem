use neomem_core::{IntakeRecord, ModelRole};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum Request {
    Status,
    Shutdown,
    Add(IntakeRecord),
    Models,
    ModelsTest { role: ModelRole },
}

impl Request {
    pub fn waits_in_spool_when_the_daemon_is_down(&self) -> bool {
        matches!(self, Self::Add(_))
    }
}
