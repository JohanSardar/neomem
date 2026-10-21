use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use neomem_core::{Choice, Decision, Judge, LlmRequest, Vector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tiny_http::{Request, Server};

use crate::transport::{Incoming, Reply};
use crate::wire_chatgpt::FakeChatgpt;
use crate::{HashEmbedder, JudgeRule, LlmRule, ScriptedJudge, ScriptedLlm};
use crate::{wire_anthropic, wire_chat, wire_responses};

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
    chatgpt: FakeChatgpt,
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
    let incoming = Incoming::read(&mut request);
    let route = incoming.route.clone();
    if !route.starts_with("/fake/") {
        shared
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall { route: route.clone(), body: incoming.body.clone() });
    }
    let delay =
        shared.latency_ms.lock().unwrap_or_else(PoisonError::into_inner).get(&route).copied().unwrap_or(0);
    thread::sleep(Duration::from_millis(delay));
    let body = &incoming.body;
    let reply = match route.as_str() {
        "/v1/embeddings" => Reply::ok(embeddings(body)),
        "/v1/systemone" => Reply::ok(system_one(shared, body)),
        "/v1/chat/completions" => wire_chat::reply(&shared.llm, body),
        "/v1/responses" => wire_responses::reply(&shared.llm, body),
        "/v1/messages" => wire_anthropic::reply(&shared.llm, &incoming),
        "/v1/brain" => brain(shared, body),
        "/oauth/authorize" => shared.chatgpt.authorize(body),
        "/oauth/token" => shared.chatgpt.token(body),
        "/backend-api/codex/responses" if shared.chatgpt.allows(&incoming) => {
            wire_responses::reply(&shared.llm, body)
        }
        "/backend-api/codex/responses" => {
            Reply::refused(401, "the ChatGPT token is missing, unknown or expired")
        }
        "/fake/rules" => Reply::ok(add_rules(shared, incoming.body)),
        "/fake/calls" => Reply::ok(json!(*shared.calls.lock().unwrap_or_else(PoisonError::into_inner))),
        _ => Reply::Json(404, json!({ "error": format!("the fake has no route {route}") })),
    };
    reply.send(request);
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
        .map(|(index, text)| json!({ "object": "embedding", "index": index, "embedding": embedding(text, body["model"].as_str().unwrap_or("")).as_slice() }))
        .collect();
    json!({ "object": "list", "model": body["model"], "data": data })
}

fn embedding(text: &str, model: &str) -> Vector {
    let vector = HashEmbedder::vector(text);
    if model == "hash-384" {
        let mut values = vector.into_inner();
        values.rotate_left(73);
        values.resize(384, 0.0);
        Vector::normalized(values)
    } else {
        vector
    }
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

fn brain(shared: &Shared, body: &Value) -> Reply {
    match serde_json::from_value::<LlmRequest>(body["request"].clone()) {
        Ok(request) => Reply::ok(json!(shared.llm.answer(&request))),
        Err(error) => Reply::refused(400, &format!("the brain needs an LLM request: {error}")),
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
