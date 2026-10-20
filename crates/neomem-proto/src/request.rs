use neomem_core::{IntakeRecord, ModelRole, Source};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum Request {
    Status,
    Shutdown,
    Add(IntakeRecord),
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
