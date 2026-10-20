use std::time::Duration;

use neomem_core::ModelError;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_idle_connections_per_host(4)
        .build()
        .into()
}

pub fn post_json<Answer: DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
    bearer: Option<&str>,
    body: &impl Serialize,
    timeout: Duration,
) -> Result<Answer, ModelError> {
    let mut request = agent.post(url).header("content-type", "application/json");
    if let Some(key) = bearer {
        request = request.header("authorization", &format!("Bearer {key}"));
    }
    let mut response = request.send_json(body).map_err(|error| failure(&error, timeout))?;
    response.body_mut().read_json().map_err(|error| ModelError::InvalidOutput(error.to_string()))
}

fn failure(error: &ureq::Error, timeout: Duration) -> ModelError {
    match error {
        ureq::Error::Timeout(_) => {
            ModelError::Timeout(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX))
        }
        ureq::Error::StatusCode(status) => {
            ModelError::Unavailable(format!("the service answered HTTP {status}"))
        }
        other => ModelError::Unavailable(other.to_string()),
    }
}
