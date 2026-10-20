use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use neomem_core::{LlmMessage, LlmRequest, ToolCall, ToolSpec};
use serde_json::{Value, json};

use crate::ScriptedAnswer;
use crate::wire_chat::tool_spec;

pub const FAKE_LLM_BINARY: &str = "neomem-fake-llm";

pub fn fake_llm_binary() -> PathBuf {
    let executable = std::env::current_exe().expect("the test knows its own path");
    let directory = executable.parent().unwrap_or(Path::new("."));
    let profile_dir =
        if directory.ends_with("deps") { directory.parent().unwrap_or(directory) } else { directory };
    let binary = profile_dir.join(FAKE_LLM_BINARY);
    if !binary.exists() {
        let release = profile_dir.ends_with("release");
        let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        build.args(["build", "--quiet", "--package", "neomem-testkit", "--bin", FAKE_LLM_BINARY]);
        if release {
            build.arg("--release");
        }
        let succeeded =
            build.current_dir(env!("CARGO_MANIFEST_DIR")).status().is_ok_and(|status| status.success());
        assert!(
            succeeded && binary.exists(),
            "build the fake LLM first: cargo build -p neomem-testkit --bins"
        );
    }
    binary
}

fn ask_brain(brain: &str, via: &str, wire: &Value, request: &LlmRequest) -> io::Result<ScriptedAnswer> {
    let body = json!({ "via": via, "wire": wire, "request": request });
    let mut response = ureq::post(&format!("{brain}/v1/brain")).send_json(&body).map_err(io::Error::other)?;
    response.body_mut().read_json().map_err(io::Error::other)
}

pub fn answer_command(mut input: impl Read, mut output: impl Write, brain: &str) -> io::Result<()> {
    let mut text = String::new();
    input.read_to_string(&mut text)?;
    let wire: Value = serde_json::from_str(&text).map_err(io::Error::other)?;
    let answer = ask_brain(brain, "command", &wire, &command_request(&wire))?;
    thread::sleep(Duration::from_millis(answer.delay_ms));
    if answer.tool_calls.is_empty() {
        output.write_all(answer.text().as_bytes())?;
    } else {
        let calls: Vec<Value> = answer
            .tool_calls
            .iter()
            .map(|call| json!({ "id": call.id, "name": call.name, "arguments": call.arguments }))
            .collect();
        output.write_all(json!({ "text": answer.text(), "tool_calls": calls }).to_string().as_bytes())?;
    }
    output.flush()
}

fn command_request(wire: &Value) -> LlmRequest {
    let messages = wire["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|message| {
            let content = message["content"].as_str().unwrap_or_default().to_owned();
            match message["role"].as_str() {
                Some("assistant") => LlmMessage::Assistant {
                    text: content,
                    tool_calls: message["tool_calls"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|call| ToolCall {
                            id: call["id"].as_str().unwrap_or_default().to_owned(),
                            name: call["name"].as_str().unwrap_or_default().to_owned(),
                            arguments: call["arguments"].as_str().unwrap_or_default().to_owned(),
                        })
                        .collect(),
                },
                Some("tool") => LlmMessage::ToolResult {
                    call_id: message["tool_call_id"].as_str().unwrap_or_default().to_owned(),
                    content,
                },
                _ => LlmMessage::User(content),
            }
        })
        .collect();
    LlmRequest {
        system: wire["system"].as_str().unwrap_or_default().to_owned(),
        messages,
        response_schema: (!wire["response_schema"].is_null()).then(|| wire["response_schema"].to_string()),
        tools: wire["tools"].as_array().into_iter().flatten().map(tool_spec).collect(),
        max_output_tokens: 0,
    }
}

pub fn serve_codex_app_server(input: impl BufRead, output: impl Write, brain: &str) -> io::Result<()> {
    let mut server = FakeCodex { output, brain: brain.to_owned(), threads: HashMap::new(), counter: 0 };
    let mut initialized = false;
    for line in input.lines() {
        let Ok(message) = serde_json::from_str::<Value>(&line?) else { continue };
        let (Some(method), Some(id)) = (message["method"].as_str(), message.get("id").cloned()) else {
            continue;
        };
        match method {
            "initialize" => {
                initialized = true;
                server.respond(
                    &id,
                    &json!({ "userAgent": "fake-codex/0.0.0", "codexHome": "/tmp/fake-codex",
                    "platformFamily": "unix", "platformOs": std::env::consts::OS }),
                )?;
            }
            _ if !initialized => server.refuse(&id, -32002, "Not initialized")?,
            "thread/start" => server.start_thread(&id, &message["params"])?,
            "turn/start" => server.run_turn(&id, &message["params"])?,
            _ => server.refuse(&id, -32601, "method not found")?,
        }
    }
    Ok(())
}

struct FakeCodex<W: Write> {
    output: W,
    brain: String,
    threads: HashMap<String, Value>,
    counter: u64,
}

impl<W: Write> FakeCodex<W> {
    fn write(&mut self, message: &Value) -> io::Result<()> {
        writeln!(self.output, "{message}")?;
        self.output.flush()
    }

    fn respond(&mut self, id: &Value, result: &Value) -> io::Result<()> {
        self.write(&json!({ "id": id, "result": result }))
    }

    fn refuse(&mut self, id: &Value, code: i64, message: &str) -> io::Result<()> {
        self.write(&json!({ "id": id, "error": { "code": code, "message": message } }))
    }

    fn notify(&mut self, method: &str, params: &Value) -> io::Result<()> {
        self.write(&json!({ "method": method, "params": params }))
    }

    fn start_thread(&mut self, id: &Value, params: &Value) -> io::Result<()> {
        self.counter += 1;
        let thread_id = format!("thr_fake_{}", self.counter);
        self.threads.insert(thread_id.clone(), params.clone());
        let model = params["model"].as_str().unwrap_or("fake-codex-model");
        self.respond(
            id,
            &json!({ "thread": { "id": thread_id }, "model": model, "modelProvider": "openai" }),
        )?;
        self.notify("thread/started", &json!({ "thread": { "id": thread_id } }))
    }

    fn run_turn(&mut self, id: &Value, params: &Value) -> io::Result<()> {
        self.counter += 1;
        let thread_id = params["threadId"].as_str().unwrap_or_default().to_owned();
        let turn_id = format!("turn_fake_{}", self.counter);
        let Some(thread) = self.threads.get(&thread_id).cloned() else {
            return self.refuse(id, -32600, "unknown thread");
        };
        self.respond(
            id,
            &json!({ "turn": { "id": turn_id, "items": [], "status": "inProgress", "error": null } }),
        )?;
        let request = codex_request(&thread, params);
        let answer = ask_brain(&self.brain, "codex-app-server", params, &request)?;
        let envelope = params["outputSchema"]["properties"]["tool_calls"].is_object();
        let item_id = format!("msg_fake_{}", self.counter);
        let scope = json!({ "threadId": thread_id, "turnId": turn_id });
        thread::sleep(Duration::from_millis(answer.delay_ms));
        let deltas = if envelope { vec![envelope_text(&answer)] } else { answer.deltas.clone() };
        for (index, delta) in deltas.iter().enumerate() {
            if index > 0 {
                thread::sleep(Duration::from_millis(answer.delta_delay_ms));
            }
            self.notify(
                "item/agentMessage/delta",
                &json!({ "threadId": thread_id, "turnId": turn_id,
                "itemId": item_id, "delta": delta }),
            )?;
        }
        let item = json!({ "type": "agentMessage", "id": item_id, "text": deltas.concat() });
        self.notify(
            "item/completed",
            &json!({ "item": item, "threadId": scope["threadId"],
            "turnId": scope["turnId"], "completedAtMs": 0 }),
        )?;
        self.notify(
            "turn/completed",
            &json!({ "threadId": thread_id,
            "turn": { "id": turn_id, "items": [], "status": "completed", "error": null } }),
        )
    }
}

fn codex_request(thread: &Value, turn: &Value) -> LlmRequest {
    let text = turn["input"][0]["text"].as_str().unwrap_or_default();
    let messages = text
        .find("\n[")
        .and_then(|start| serde_json::from_str::<Vec<LlmMessage>>(&text[start + 1..]).ok())
        .unwrap_or_else(|| vec![LlmMessage::User(text.to_owned())]);
    let schema = &turn["outputSchema"];
    let tools = tools_in_envelope(schema);
    LlmRequest {
        system: thread["developerInstructions"].as_str().unwrap_or_default().to_owned(),
        messages,
        response_schema: (!schema.is_null() && tools.is_empty()).then(|| schema.to_string()),
        tools,
        max_output_tokens: 0,
    }
}

fn tools_in_envelope(schema: &Value) -> Vec<ToolSpec> {
    schema["properties"]["tool_calls"]["items"]["anyOf"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|shape| ToolSpec {
            name: shape["properties"]["name"]["enum"][0].as_str().unwrap_or_default().to_owned(),
            description: String::new(),
            parameters_schema: shape["properties"]["arguments"].to_string(),
        })
        .collect()
}

fn envelope_text(answer: &ScriptedAnswer) -> String {
    let calls: Vec<Value> = answer
        .tool_calls
        .iter()
        .map(|call| {
            let arguments: Value =
                serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!(call.arguments));
            json!({ "name": call.name, "arguments": arguments })
        })
        .collect();
    json!({ "text": answer.text(), "tool_calls": calls }).to_string()
}
