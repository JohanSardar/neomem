use std::io::BufReader;
use std::time::Duration;

use neomem_core::ModelError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::Body;
use ureq::http::Response;

const ERROR_EXCERPT_CHARS: usize = 300;

pub type Headers<'call> = &'call [(&'static str, String)];

pub type BodyLines = BufReader<ureq::BodyReader<'static>>;

pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_idle_connections_per_host(4)
        .http_status_as_error(false)
        .build()
        .into()
}

pub fn bearer(key: Option<&str>) -> Vec<(&'static str, String)> {
    key.map(|key| vec![("authorization", format!("Bearer {key}"))]).unwrap_or_default()
}

pub fn post_json<Answer: DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
    bearer_key: Option<&str>,
    body: &impl Serialize,
    timeout: Duration,
) -> Result<Answer, ModelError> {
    let response = send(agent, url, &bearer(bearer_key), body, timeout)?;
    read_json(response)
}

pub fn post_form<Answer: DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
    form: &[(&str, &str)],
    timeout: Duration,
) -> Result<Answer, ModelError> {
    let sent = agent.post(url).send_form(form.iter().copied());
    read_json(checked(sent, timeout)?)
}

pub fn post_stream(
    agent: &ureq::Agent,
    url: &str,
    headers: Headers<'_>,
    body: &impl Serialize,
    timeout: Duration,
) -> Result<(Option<String>, BodyLines), ModelError> {
    let response = send(agent, url, headers, body, timeout)?;
    let content_type =
        response.headers().get("content-type").and_then(|value| value.to_str().ok()).map(str::to_owned);
    Ok((content_type, BufReader::new(response.into_body().into_reader())))
}

fn send(
    agent: &ureq::Agent,
    url: &str,
    headers: Headers<'_>,
    body: &impl Serialize,
    timeout: Duration,
) -> Result<Response<Body>, ModelError> {
    let mut request = agent.post(url).header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    checked(request.send_json(body), timeout)
}

fn read_json<Answer: DeserializeOwned>(mut response: Response<Body>) -> Result<Answer, ModelError> {
    response.body_mut().read_json().map_err(|error| ModelError::InvalidOutput(error.to_string()))
}

fn checked(
    sent: Result<Response<Body>, ureq::Error>,
    timeout: Duration,
) -> Result<Response<Body>, ModelError> {
    let mut response = sent.map_err(|error| failure(&error, timeout))?;
    let status = response.status().as_u16();
    if status < 400 {
        return Ok(response);
    }
    let text = response.body_mut().read_to_string().unwrap_or_default();
    let excerpt: String =
        text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(ERROR_EXCERPT_CHARS).collect();
    Err(ModelError::Unavailable(format!("the service answered HTTP {status}: {excerpt}")))
}

pub fn timed_out(timeout: Duration) -> ModelError {
    ModelError::Timeout(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX))
}

fn failure(error: &ureq::Error, timeout: Duration) -> ModelError {
    match error {
        ureq::Error::Timeout(_) => timed_out(timeout),
        other => ModelError::Unavailable(other.to_string()),
    }
}
