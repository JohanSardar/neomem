use serde::{Deserialize, Serialize};

use crate::Request;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol: u32,
    pub request: Request,
}

#[derive(Debug, Deserialize)]
pub struct IncomingEnvelope {
    pub protocol: u32,
    pub request: serde_json::Value,
}

impl Envelope {
    pub fn current(request: Request) -> Self {
        Self { protocol: PROTOCOL_VERSION, request }
    }
}

impl IncomingEnvelope {
    pub fn into_request(self) -> Result<Request, serde_json::Error> {
        serde_json::from_value(self.request)
    }

    pub fn speaks_current_protocol(&self) -> bool {
        self.protocol == PROTOCOL_VERSION
    }

    pub fn asks_for_shutdown(&self) -> bool {
        self.request.get("command").and_then(serde_json::Value::as_str) == Some("shutdown")
    }
}
