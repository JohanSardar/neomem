use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum Request {
    Status,
    Shutdown,
}
