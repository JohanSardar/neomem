use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{Effort, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolCall, ToolSpec};
use serde_json::{Value, json};

use crate::http;
use crate::json_schema;
use crate::llm::{self, LlmSettings, Wire};

pub const DEFAULT_COMMAND: &[&str] = &["codex", "app-server"];

const TRANSCRIPT: &str = include_str!("../prompts/transcript.md");
const TOOLS_AS_JSON: &str = include_str!("../prompts/tools_as_json.md");

#[derive(Debug)]
pub struct CodexLlm {
    program: Vec<String>,
    model: Option<String>,
    effort: Option<Effort>,
    timeout: Duration,
    server: Mutex<Option<AppServer>>,
}

impl CodexLlm {
    pub fn new(program: Option<&[String]>, settings: &LlmSettings) -> Self {
        let program = program.filter(|program| !program.is_empty()).map_or_else(
            || DEFAULT_COMMAND.iter().map(|part| (*part).to_owned()).collect(),
            <[String]>::to_vec,
        );
        Self {
            program,
            model: settings.model.clone(),
            effort: settings.effort,
            timeout: settings.timeout,
            server: Mutex::new(None),
        }
    }

    fn turn(
        &self,
        server: &mut AppServer,
        request: &LlmRequest,
        on_text: &mut dyn FnMut(&str),
        deadline: Instant,
    ) -> Result<LlmReply, ModelError> {
        let envelope = tool_envelope(&request.tools)?;
        let mut thread = json!({ "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": true,
            "cwd": std::env::temp_dir() });
        if let Some(model) = &self.model {
            thread["model"] = json!(model);
        }
        let instructions = developer_instructions(request)?;
        if !instructions.is_empty() {
            thread["developerInstructions"] = json!(instructions);
        }
        let started = server.call("thread/start", &thread, deadline)?;
        let thread_id = started["thread"]["id"]
            .as_str()
            .ok_or_else(|| ModelError::InvalidOutput("thread/start returned no thread ID".to_owned()))?
            .to_owned();
        let mut turn = json!({ "threadId": thread_id,
            "input": [{ "type": "text", "text": transcript(request), "text_elements": [] }] });
        if let Some(effort) = self.effort {
            turn["effort"] = json!(effort.name());
        }
        let schema = match (&envelope, &request.response_schema) {
            (Some(envelope), _) => Some(envelope.clone()),
            (None, Some(schema)) => Some(llm::parse_schema(schema)?),
            (None, None) => None,
        };
        let streams = schema.is_none();
        if let Some(schema) = schema {
            turn["outputSchema"] = schema;
        }
        let pending = server.request("turn/start", &turn)?;
        let mut streamed = String::new();
        let mut last_message = String::new();
        let mut failure = None;
        loop {
            let message = server.next(deadline)?;
            if message.get("method").is_none() {
                if message["id"] == pending
                    && let Some(error) = message.get("error")
                {
                    return Err(rpc_failure("turn/start", error));
                }
                continue;
            }
            server.decline(&message)?;
            let params = &message["params"];
            match message["method"].as_str().unwrap_or_default() {
                "item/agentMessage/delta" if params["threadId"] == thread_id => {
                    let delta = params["delta"].as_str().unwrap_or_default();
                    streamed.push_str(delta);
                    if streams {
                        on_text(delta);
                    }
                }
                "item/completed" if params["item"]["type"] == "agentMessage" => {
                    params["item"]["text"].as_str().unwrap_or_default().clone_into(&mut last_message);
                }
                "error" if params["willRetry"] != true => {
                    failure = params["error"]["message"].as_str().map(str::to_owned);
                }
                "turn/completed" if params["threadId"] == thread_id => {
                    if params["turn"]["status"] == "completed" {
                        break;
                    }
                    let reason = failure
                        .or_else(|| params["turn"]["error"]["message"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| format!("the codex turn ended {}", params["turn"]["status"]));
                    return Err(ModelError::Unavailable(reason));
                }
                _ => {}
            }
        }
        if envelope.is_some() {
            let reply = from_envelope(&last_message)?;
            if !reply.text.is_empty() {
                on_text(&reply.text);
            }
            return Ok(reply);
        }
        if streams && !streamed.is_empty() {
            return Ok(LlmReply { text: streamed, tool_calls: Vec::new() });
        }
        if !last_message.is_empty() {
            on_text(&last_message);
        }
        Ok(LlmReply { text: last_message, tool_calls: Vec::new() })
    }
}

impl Wire for CodexLlm {
    fn has_schema_mode(&self) -> bool {
        true
    }

    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let deadline = Instant::now() + self.timeout;
        let mut slot = self.server.lock().unwrap_or_else(PoisonError::into_inner);
        let server = match slot.as_mut() {
            Some(server) => server,
            None => slot.insert(AppServer::start(&self.program, self.timeout, deadline)?),
        };
        let outcome = self.turn(server, request, on_text, deadline);
        if outcome.is_err() {
            *slot = None;
        }
        outcome
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("codex:{}", self.model.as_deref().unwrap_or("default")))
    }
}

fn developer_instructions(request: &LlmRequest) -> Result<String, ModelError> {
    if request.tools.is_empty() {
        return Ok(request.system.clone());
    }
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            Ok(json!({ "name": tool.name, "description": tool.description, "parameters": llm::tool_parameters(tool)? }))
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    let listed = TOOLS_AS_JSON.trim().replace("{tools}", &Value::Array(tools).to_string());
    Ok([request.system.trim(), listed.as_str()]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn transcript(request: &LlmRequest) -> String {
    match request.messages.as_slice() {
        [LlmMessage::User(text)] => text.clone(),
        messages => {
            let json = serde_json::to_string(messages).unwrap_or_default();
            TRANSCRIPT.trim().replace("{transcript}", &json)
        }
    }
}

fn tool_envelope(tools: &[ToolSpec]) -> Result<Option<Value>, ModelError> {
    if tools.is_empty() {
        return Ok(None);
    }
    let shapes = tools
        .iter()
        .map(|tool| {
            let parameters = llm::tool_parameters(tool)?;
            let arguments = if json_schema::is_strict(&parameters) { parameters } else { json!({ "type": "string" }) };
            Ok(json!({ "type": "object", "additionalProperties": false, "required": ["name", "arguments"],
                "properties": { "name": { "type": "string", "enum": [tool.name] }, "arguments": arguments } }))
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    Ok(Some(json!({ "type": "object", "additionalProperties": false, "required": ["text", "tool_calls"],
        "properties": { "text": { "type": "string" }, "tool_calls": { "type": "array", "items": { "anyOf": shapes } } } })))
}

fn from_envelope(text: &str) -> Result<LlmReply, ModelError> {
    let envelope: Value = serde_json::from_str(text.trim()).map_err(|error| {
        ModelError::InvalidOutput(format!("codex did not answer with the tool JSON: {error}"))
    })?;
    let tool_calls = envelope["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, call)| ToolCall {
            id: format!("call_{}", index + 1),
            name: call["name"].as_str().unwrap_or_default().to_owned(),
            arguments: match &call["arguments"] {
                Value::String(arguments) => arguments.clone(),
                other => other.to_string(),
            },
        })
        .collect();
    Ok(LlmReply { text: envelope["text"].as_str().unwrap_or_default().to_owned(), tool_calls })
}

fn rpc_failure(method: &str, error: &Value) -> ModelError {
    ModelError::Unavailable(format!(
        "{method}: {}",
        error["message"].as_str().unwrap_or("the app server refused")
    ))
}

#[derive(Debug)]
struct AppServer {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Value>,
    next_id: u64,
    timeout: Duration,
}

impl AppServer {
    fn start(program: &[String], timeout: Duration, deadline: Instant) -> Result<Self, ModelError> {
        let mut child = Command::new(&program[0])
            .args(&program[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                ModelError::Unavailable(format!("cannot start {}: {error}", program.join(" ")))
            })?;
        let pipes = child.stdin.take().zip(child.stdout.take());
        let Some((input, output)) = pipes else {
            let _ = child.kill();
            return Err(ModelError::Unavailable("the codex app-server has no stdio".to_owned()));
        };
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str::<Value>(&line)
                    && sender.send(message).is_err()
                {
                    break;
                }
            }
        });
        let mut server = Self { child, input, messages, next_id: 0, timeout };
        let client = json!({ "clientInfo": { "name": "neomem", "title": "neomem", "version": env!("CARGO_PKG_VERSION") } });
        server.call("initialize", &client, deadline)?;
        server.write(&json!({ "method": "initialized" }))?;
        Ok(server)
    }

    fn write(&mut self, message: &Value) -> Result<(), ModelError> {
        writeln!(self.input, "{message}").and_then(|()| self.input.flush()).map_err(|error| {
            ModelError::Unavailable(format!("the codex app-server stopped listening: {error}"))
        })
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<u64, ModelError> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({ "id": id, "method": method, "params": params }))?;
        Ok(id)
    }

    fn call(&mut self, method: &str, params: &Value, deadline: Instant) -> Result<Value, ModelError> {
        let id = self.request(method, params)?;
        loop {
            let message = self.next(deadline)?;
            if message.get("method").is_some() {
                self.decline(&message)?;
            } else if message["id"] == id {
                return match message.get("error") {
                    Some(error) => Err(rpc_failure(method, error)),
                    None => Ok(message["result"].clone()),
                };
            }
        }
    }

    fn next(&self, deadline: Instant) -> Result<Value, ModelError> {
        match self.messages.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(message) => Ok(message),
            Err(RecvTimeoutError::Timeout) => Err(http::timed_out(self.timeout)),
            Err(RecvTimeoutError::Disconnected) => {
                Err(ModelError::Unavailable("the codex app-server exited".to_owned()))
            }
        }
    }

    fn decline(&mut self, message: &Value) -> Result<(), ModelError> {
        match message.get("id") {
            Some(id) => self.write(&json!({ "id": id,
                "error": { "code": -32601, "message": "neomem grants no requests from the app server" } })),
            None => Ok(()),
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
