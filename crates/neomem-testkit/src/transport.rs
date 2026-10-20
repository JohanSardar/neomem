use std::collections::BTreeMap;
use std::io::Write;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tiny_http::{Header, Request, Response};

#[derive(Debug)]
pub struct Incoming {
    pub route: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

impl Incoming {
    pub fn read(request: &mut Request) -> Self {
        let url = request.url().to_owned();
        let (route, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
        let headers: BTreeMap<String, String> = request
            .headers()
            .iter()
            .map(|header| {
                (header.field.as_str().as_str().to_ascii_lowercase(), header.value.as_str().to_owned())
            })
            .collect();
        let mut text = String::new();
        let _ = request.as_reader().read_to_string(&mut text);
        let is_form = headers
            .get("content-type")
            .is_some_and(|kind| kind.starts_with("application/x-www-form-urlencoded"));
        let body = if is_form {
            form_fields(&text)
        } else if text.trim().is_empty() {
            if query.is_empty() { Value::Null } else { form_fields(query) }
        } else {
            serde_json::from_str(&text).unwrap_or(Value::Null)
        };
        Self { route: route.to_owned(), headers, body }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

pub fn form_fields(text: &str) -> Value {
    Value::Object(
        form_urlencoded::parse(text.as_bytes())
            .map(|(name, value)| (name.into_owned(), Value::String(value.into_owned())))
            .collect(),
    )
}

#[derive(Debug)]
pub struct TimedEvent {
    pub after_ms: u64,
    pub text: String,
}

impl TimedEvent {
    pub fn data(after_ms: u64, data: &Value) -> Self {
        Self { after_ms, text: format!("data: {data}\n\n") }
    }

    pub fn named(after_ms: u64, name: &str, data: &Value) -> Self {
        Self { after_ms, text: format!("event: {name}\ndata: {data}\n\n") }
    }

    pub fn done() -> Self {
        Self { after_ms: 0, text: "data: [DONE]\n\n".to_owned() }
    }
}

#[derive(Debug)]
pub enum Reply {
    Json(u16, Value),
    Events(Vec<TimedEvent>),
    Redirect(String),
}

impl Reply {
    pub fn ok(body: Value) -> Self {
        Self::Json(200, body)
    }

    pub fn refused(status: u16, message: &str) -> Self {
        Self::Json(status, serde_json::json!({ "error": { "message": message } }))
    }

    pub fn send(self, request: Request) {
        match self {
            Self::Json(status, body) => {
                let mut response = Response::from_string(body.to_string()).with_status_code(status);
                if let Ok(header) = Header::from_bytes("content-type", "application/json") {
                    response = response.with_header(header);
                }
                let _ = request.respond(response);
            }
            Self::Events(events) => stream(request, events),
            Self::Redirect(location) => {
                let mut response = Response::empty(302);
                if let Ok(header) = Header::from_bytes("location", location.as_bytes()) {
                    response = response.with_header(header);
                }
                let _ = request.respond(response);
            }
        }
    }
}

fn stream(request: Request, events: Vec<TimedEvent>) {
    let mut writer = request.into_writer();
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ntransfer-encoding: chunked\r\n\r\n";
    if writer.write_all(head.as_bytes()).and_then(|()| writer.flush()).is_err() {
        return;
    }
    for event in events {
        thread::sleep(Duration::from_millis(event.after_ms));
        let chunk = format!("{:x}\r\n{}\r\n", event.text.len(), event.text);
        if writer.write_all(chunk.as_bytes()).and_then(|()| writer.flush()).is_err() {
            return;
        }
    }
    let _ = writer.write_all(b"0\r\n\r\n").and_then(|()| writer.flush());
}
