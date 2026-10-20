use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use neomem_core::{Choice, Decision, Judge, Llm, LlmMessage, LlmRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tiny_http::{Header, Request, Response, Server};

use crate::{HashEmbedder, JudgeRule, LlmRule, ScriptedJudge, ScriptedLlm};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FakeRules {
    #[serde(default)]
    pub judge: Vec<JudgeRule>,
    #[serde(default)]
    pub llm: Vec<LlmRule>,
    #[serde(default)]
    pub latency_ms: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedCall {
    pub route: String,
    pub body: Value,
}

#[derive(Debug, Default)]
struct Shared {
    judge: ScriptedJudge,
    llm: ScriptedLlm,
    calls: Mutex<Vec<RecordedCall>>,
    latency_ms: Mutex<BTreeMap<String, u64>>,
}

pub struct FakeModelServer {
    server: Arc<Server>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for FakeModelServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("FakeModelServer").field("address", &self.address()).finish_non_exhaustive()
    }
}

impl FakeModelServer {
    pub fn bind(port: u16) -> io::Result<Self> {
        let server = Server::http(("127.0.0.1", port)).map_err(io::Error::other)?;
        Ok(Self { server: Arc::new(server), shared: Arc::new(Shared::default()) })
    }

    pub fn address(&self) -> Option<SocketAddr> {
        self.server.server_addr().to_ip()
    }

    pub fn serve_forever(&self) {
        for request in self.server.incoming_requests() {
            let shared = Arc::clone(&self.shared);
            thread::spawn(move || answer(&shared, request));
        }
    }

    pub fn serve_in_background(self) -> Option<SocketAddr> {
        let address = self.address();
        thread::spawn(move || self.serve_forever());
        address
    }
}

fn answer(shared: &Shared, mut request: Request) {
    let route = request.url().split('?').next().unwrap_or_default().to_owned();
    let mut text = String::new();
    let body: Value = match request.as_reader().read_to_string(&mut text) {
        Ok(_) if text.trim().is_empty() => Value::Null,
        Ok(_) => serde_json::from_str(&text).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };
    if !route.starts_with("/fake/") {
        shared
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall { route: route.clone(), body: body.clone() });
    }
    let delay =
        shared.latency_ms.lock().unwrap_or_else(PoisonError::into_inner).get(&route).copied().unwrap_or(0);
    thread::sleep(Duration::from_millis(delay));
    let (status, reply) = match route.as_str() {
        "/v1/embeddings" => (200, embeddings(&body)),
        "/v1/systemone" => (200, system_one(shared, &body)),
        "/v1/chat/completions" => chat(shared, &body),
        "/fake/rules" => (200, add_rules(shared, body)),
        "/fake/calls" => (200, json!(*shared.calls.lock().unwrap_or_else(PoisonError::into_inner))),
        _ => (404, json!({ "error": format!("the fake has no route {route}") })),
    };
    let header = Header::from_bytes("content-type", "application/json").ok();
    let mut response = Response::from_string(reply.to_string()).with_status_code(status);
    if let Some(header) = header {
        response = response.with_header(header);
    }
    let _ = request.respond(response);
}

fn embeddings(body: &Value) -> Value {
    let texts: Vec<String> = match &body["input"] {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    let data: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| json!({ "object": "embedding", "index": index, "embedding": HashEmbedder::vector(text).as_slice() }))
        .collect();
    json!({ "object": "list", "model": body["model"], "data": data })
}

fn system_one(shared: &Shared, body: &Value) -> Value {
    let state = body["state"].as_str().map_or_else(|| body["state"].to_string(), str::to_owned);
    let empty = serde_json::Map::new();
    let questions = body["questions"].as_object().unwrap_or(&empty);
    let answers: serde_json::Map<String, Value> = questions
        .iter()
        .map(|(id, question)| (id.clone(), answer_question(&shared.judge, &state, question)))
        .collect();
    json!({ "model": "jev-fake", "answers": answers, "usage": { "input_tokens": state.len() / 4, "output_tokens": 0 } })
}

fn answer_question(judge: &ScriptedJudge, state: &str, question: &Value) -> Value {
    let instructions = question["instructions"].as_str().unwrap_or_default();
    if question["type"] == "choice" {
        answer_choice(judge, state, instructions, question)
    } else {
        answer_noul(judge, state, instructions)
    }
}

fn answer_choice(judge: &ScriptedJudge, state: &str, instructions: &str, question: &Value) -> Value {
    let empty = serde_json::Map::new();
    let criteria = question["criteria"].as_object().unwrap_or(&empty);
    let options: Vec<Choice<'_>> = criteria
        .iter()
        .map(|(label, meaning)| Choice { label, meaning: meaning.as_str().unwrap_or_default() })
        .collect();
    let decision = judge.decide(state, instructions, &options).unwrap_or(Decision { probabilities: vec![] });
    let probabilities: serde_json::Map<String, Value> = options
        .iter()
        .zip(&decision.probabilities)
        .map(|(option, probability)| (option.label.to_owned(), json!(probability)))
        .collect();
    let choice = options.get(decision.choice()).map_or("", |option| option.label);
    json!({ "type": "choice", "choice": choice, "confidence": decision.confidence(), "probabilities": probabilities })
}

fn answer_noul(judge: &ScriptedJudge, state: &str, instructions: &str) -> Value {
    let (question_text, candidate) =
        instructions.split_once("\nCandidate: ").unwrap_or((instructions, instructions));
    let noul = judge
        .score(state, question_text, &[candidate])
        .ok()
        .and_then(|scores| scores.first().copied())
        .unwrap_or(0.0);
    json!({ "type": "noul", "noul": noul })
}

fn chat(shared: &Shared, body: &Value) -> (u16, Value) {
    let empty = Vec::new();
    let messages = body["messages"].as_array().unwrap_or(&empty);
    let text_of = |role: &str| -> String {
        messages
            .iter()
            .filter(|message| message["role"] == role)
            .filter_map(|message| message["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let request = LlmRequest {
        system: text_of("system"),
        messages: vec![LlmMessage::User(text_of("user"))],
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: 0,
    };
    match shared.llm.complete(&request, &mut |_| {}) {
        Ok(reply) => (
            200,
            json!({
                "id": "chatcmpl-fake",
                "object": "chat.completion",
                "model": body["model"],
                "choices": [{ "index": 0, "message": { "role": "assistant", "content": reply.text }, "finish_reason": "stop" }],
            }),
        ),
        Err(error) => (503, json!({ "error": { "message": error.to_string() } })),
    }
}

fn add_rules(shared: &Shared, body: Value) -> Value {
    let rules: FakeRules = serde_json::from_value(body).unwrap_or_default();
    let added = rules.judge.len() + rules.llm.len();
    shared.judge.add_rules(rules.judge);
    shared.llm.add_rules(rules.llm);
    shared.latency_ms.lock().unwrap_or_else(PoisonError::into_inner).extend(rules.latency_ms);
    json!({ "added": added })
}
