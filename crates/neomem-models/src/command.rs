use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use neomem_core::{Effort, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolCall};
use serde_json::{Value, json};

use crate::http;
use crate::llm::{self, LlmSettings, Wire};

const POLL: Duration = Duration::from_millis(2);
const STDERR_EXCERPT_CHARS: usize = 300;

#[derive(Debug)]
pub struct CommandLlm {
    program: Vec<String>,
    model: Option<String>,
    effort: Option<Effort>,
    timeout: Duration,
}

struct Finished {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl CommandLlm {
    pub fn new(program: &[String], settings: &LlmSettings) -> Result<Self, ModelError> {
        if program.is_empty() {
            return Err(ModelError::NotConfigured(
                "a command provider needs cmd = [\"program\", ...]".to_owned(),
            ));
        }
        Ok(Self {
            program: program.to_vec(),
            model: settings.model.clone(),
            effort: settings.effort,
            timeout: settings.timeout,
        })
    }

    fn request_json(&self, request: &LlmRequest) -> Result<Value, ModelError> {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                Ok(json!({ "name": tool.name, "description": tool.description, "parameters": llm::tool_parameters(tool)? }))
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        let schema = request.response_schema.as_deref().map(llm::parse_schema).transpose()?;
        Ok(json!({
            "model": self.model,
            "effort": self.effort.map(Effort::name),
            "system": request.system,
            "messages": request.messages.iter().map(message_json).collect::<Vec<_>>(),
            "response_schema": schema,
            "tools": tools,
            "max_output_tokens": request.max_output_tokens,
        }))
    }

    fn run(&self, input: Vec<u8>) -> Result<Finished, ModelError> {
        let name = &self.program[0];
        let mut child = Command::new(name)
            .args(&self.program[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ModelError::Unavailable(format!("cannot start {name}: {error}")))?;
        let stdin = child.stdin.take();
        let writer = thread::spawn(move || stdin.map(|mut pipe| pipe.write_all(&input)));
        let stdout = read_in_background(child.stdout.take());
        let stderr = read_in_background(child.stderr.take());
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(http::timed_out(self.timeout));
                }
                Err(error) => return Err(ModelError::Unavailable(format!("waiting for {name}: {error}"))),
            }
        };
        let _ = writer.join();
        Ok(Finished {
            status,
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        })
    }
}

fn read_in_background<Pipe: Read + Send + 'static>(pipe: Option<Pipe>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

fn message_json(message: &LlmMessage) -> Value {
    match message {
        LlmMessage::User(text) => json!({ "role": "user", "content": text }),
        LlmMessage::Assistant { text, tool_calls } => {
            let calls: Vec<Value> = tool_calls
                .iter()
                .map(|call| json!({ "id": call.id, "name": call.name, "arguments": call.arguments }))
                .collect();
            json!({ "role": "assistant", "content": text, "tool_calls": calls })
        }
        LlmMessage::ToolResult { call_id, content } => {
            json!({ "role": "tool", "tool_call_id": call_id, "content": content })
        }
    }
}

fn parse_answer(stdout: &str) -> LlmReply {
    let trimmed = stdout.trim();
    let envelope = serde_json::from_str::<Value>(trimmed).ok().filter(|value| {
        value.as_object().is_some_and(|fields| {
            !fields.is_empty() && fields.keys().all(|key| key == "text" || key == "tool_calls")
        })
    });
    let Some(envelope) = envelope else {
        return LlmReply { text: trimmed.to_owned(), tool_calls: Vec::new() };
    };
    let tool_calls = envelope["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, call)| ToolCall {
            id: call["id"].as_str().map_or_else(|| format!("call_{}", index + 1), str::to_owned),
            name: call["name"].as_str().unwrap_or_default().to_owned(),
            arguments: match &call["arguments"] {
                Value::String(text) => text.clone(),
                Value::Null => "{}".to_owned(),
                other => other.to_string(),
            },
        })
        .collect();
    LlmReply { text: envelope["text"].as_str().unwrap_or_default().to_owned(), tool_calls }
}

impl Wire for CommandLlm {
    fn has_schema_mode(&self) -> bool {
        false
    }

    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let input = self.request_json(request)?.to_string().into_bytes();
        let finished = self.run(input)?;
        if !finished.status.success() {
            let stderr = String::from_utf8_lossy(&finished.stderr);
            let excerpt: String = stderr.trim().chars().take(STDERR_EXCERPT_CHARS).collect();
            return Err(ModelError::Unavailable(format!(
                "{} exited with {}: {excerpt}",
                self.program[0], finished.status
            )));
        }
        let reply = parse_answer(&String::from_utf8_lossy(&finished.stdout));
        if !reply.text.is_empty() {
            on_text(&reply.text);
        }
        Ok(reply)
    }

    fn model_id(&self) -> ModelId {
        let program = Path::new(&self.program[0])
            .file_name()
            .map_or_else(|| self.program[0].clone(), |name| name.to_string_lossy().into_owned());
        ModelId::new(format!("command:{}", self.model.as_deref().unwrap_or(&program)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_stdout_is_the_answer_and_an_envelope_carries_tool_calls() {
        assert_eq!(parse_answer("  hello\n").text, "hello");
        let reply =
            parse_answer(r#"{"text":"","tool_calls":[{"name":"lookup","arguments":{"city":"Paris"}}]}"#);
        assert_eq!(reply.tool_calls[0].id, "call_1");
        assert_eq!(reply.tool_calls[0].arguments, r#"{"city":"Paris"}"#);
        assert_eq!(parse_answer(r#"{"answer":"ok"}"#).text, r#"{"answer":"ok"}"#);
    }

    #[test]
    fn a_command_that_runs_too_long_is_stopped_at_the_timeout() {
        let settings =
            LlmSettings { model: None, effort: None, fast: false, timeout: Duration::from_millis(100) };
        let slow = CommandLlm::new(&["sleep".to_owned(), "5".to_owned()], &settings).unwrap();
        let request = LlmRequest {
            system: String::new(),
            messages: Vec::new(),
            response_schema: None,
            tools: Vec::new(),
            max_output_tokens: 0,
        };
        let started = Instant::now();
        assert!(matches!(slow.send(&request, &mut |_| {}), Err(ModelError::Timeout(100))));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
