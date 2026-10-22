use std::io::BufReader;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::ModelError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::Body;
use ureq::http::Response;

const ERROR_EXCERPT_CHARS: usize = 300;
const BRIEFLY_UNAVAILABLE: [u16; 3] = [502, 503, 504];
const RETRY_PAUSES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

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
    let started = Instant::now();
    let mut pauses = RETRY_PAUSES.iter();
    loop {
        let mut request = agent.post(url).header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let sent = request.send_json(body);
        let worth_retrying = match &sent {
            Ok(response) => BRIEFLY_UNAVAILABLE.contains(&response.status().as_u16()),
            Err(error) => matches!(error, ureq::Error::Io(_) | ureq::Error::ConnectionFailed),
        };
        match pauses.next() {
            Some(pause) if worth_retrying && started.elapsed() + *pause < timeout => thread::sleep(*pause),
            _ => return checked(sent, timeout),
        }
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn serving(statuses: Vec<u16>) -> (String, Arc<AtomicUsize>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = format!("http://{}", server.server_addr().to_ip().unwrap());
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen);
        thread::spawn(move || {
            for request in server.incoming_requests() {
                let index = counter.fetch_add(1, Ordering::SeqCst);
                let status = statuses.get(index).copied().unwrap_or(200);
                let body = if status == 200 { r#"{"ok":true}"# } else { "upstream connect error" };
                request.respond(tiny_http::Response::from_string(body).with_status_code(status)).unwrap();
            }
        });
        (address, seen)
    }

    fn post(address: &str, timeout: Duration) -> Result<serde_json::Value, ModelError> {
        post_json(&agent(timeout), address, None, &serde_json::json!({}), timeout)
    }

    #[test]
    fn a_briefly_unavailable_service_is_asked_again_before_failing_the_call() {
        let (address, seen) = serving(vec![503]);
        assert_eq!(post(&address, Duration::from_secs(30)).unwrap(), serde_json::json!({"ok": true}));
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_refused_request_is_not_repeated_and_retries_stay_within_the_timeout() {
        let (address, seen) = serving(vec![400]);
        assert!(matches!(post(&address, Duration::from_secs(30)), Err(ModelError::Unavailable(_))));
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        let (address, seen) = serving(vec![503; 9]);
        let failed = post(&address, Duration::from_secs(3));
        assert!(matches!(&failed, Err(ModelError::Unavailable(message)) if message.contains("HTTP 503")));
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }
}
