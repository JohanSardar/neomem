use std::io::Read;
use std::time::Duration;

use neomem_core::{Effort, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolCall};
use serde_json::{Value, json};

use crate::http::{self, BodyLines};
use crate::json_schema;
use crate::llm::{self, LlmSettings, Wire};
use crate::sse::{self, Flow};

#[derive(Debug)]
pub struct OpenAiResponses {
    agent: ureq::Agent,
    url: String,
    key: Option<String>,
    model: String,
    effort: Option<Effort>,
    timeout: Duration,
}

impl OpenAiResponses {
    pub fn new(base_url: &str, key: Option<String>, settings: &LlmSettings) -> Result<Self, ModelError> {
        Ok(Self {
            agent: http::agent(settings.timeout),
            url: format!("{}/responses", base_url.trim_end_matches('/')),
            key,
            model: settings.required_model("openai")?,
            effort: settings.effort,
            timeout: settings.timeout,
        })
    }
}

impl Wire for OpenAiResponses {
    fn has_schema_mode(&self) -> bool {
        true
    }

    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let mut body = body(&self.model, self.effort, request)?;
        if request.max_output_tokens > 0 {
            body["max_output_tokens"] = json!(request.max_output_tokens);
        }
        let headers = http::bearer(self.key.as_deref());
        let (content_type, lines) = http::post_stream(&self.agent, &self.url, &headers, &body, self.timeout)?;
        read_reply(content_type, lines, self.timeout, on_text)
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("openai:{}", self.model))
    }
}

pub fn body(model: &str, effort: Option<Effort>, request: &LlmRequest) -> Result<Value, ModelError> {
    let mut body = json!({
        "model": model,
        "instructions": request.system,
        "input": request.messages.iter().flat_map(input_items).collect::<Vec<_>>(),
        "stream": true,
        "store": false,
    });
    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                Ok(json!({ "type": "function", "name": tool.name, "description": tool.description,
                    "parameters": llm::tool_parameters(tool)?, "strict": false }))
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        body["tools"] = Value::Array(tools);
    }
    if let Some(schema) = &request.response_schema {
        let schema = llm::parse_schema(schema)?;
        let strict = json_schema::is_strict(&schema);
        body["text"] = json!({ "format": { "type": "json_schema", "name": "answer", "schema": schema, "strict": strict } });
    }
    if let Some(effort) = effort {
        body["reasoning"] = json!({ "effort": effort.name() });
    }
    Ok(body)
}

fn input_items(message: &LlmMessage) -> Vec<Value> {
    match message {
        LlmMessage::User(text) => {
            vec![
                json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }),
            ]
        }
        LlmMessage::Assistant { text, tool_calls } => {
            let spoken = (!text.is_empty()).then(|| {
                json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text }] })
            });
            let calls = tool_calls.iter().map(|call| {
                json!({ "type": "function_call", "call_id": call.id, "name": call.name, "arguments": call.arguments })
            });
            spoken.into_iter().chain(calls).collect()
        }
        LlmMessage::ToolResult { call_id, content } => {
            vec![json!({ "type": "function_call_output", "call_id": call_id, "output": content })]
        }
    }
}

pub fn read_reply(
    content_type: Option<String>,
    mut lines: BodyLines,
    timeout: Duration,
    on_text: &mut dyn FnMut(&str),
) -> Result<LlmReply, ModelError> {
    if content_type.is_some_and(|kind| kind.contains("application/json")) {
        let mut text = String::new();
        lines.read_to_string(&mut text).map_err(|error| ModelError::Unavailable(error.to_string()))?;
        let response: Value = serde_json::from_str(&text)
            .map_err(|error| ModelError::InvalidOutput(format!("the response is not JSON: {error}")))?;
        let reply = from_output(&response["output"]);
        if !reply.text.is_empty() {
            on_text(&reply.text);
        }
        return Ok(reply);
    }
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut finished = None;
    sse::read_events(lines, timeout, |event| {
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|error| ModelError::InvalidOutput(format!("a stream event is not JSON: {error}")))?;
        match data["type"].as_str().unwrap_or(event.name.as_str()) {
            "response.output_text.delta" => {
                let delta = data["delta"].as_str().unwrap_or_default();
                text.push_str(delta);
                on_text(delta);
            }
            "response.output_item.done" if data["item"]["type"] == "function_call" => {
                tool_calls.push(tool_call(&data["item"]));
            }
            "response.completed" | "response.incomplete" => {
                finished = Some(data["response"]["output"].clone());
                return Ok(Flow::Done);
            }
            "response.failed" => return Err(failure(&data["response"]["error"])),
            "error" => return Err(failure(&data)),
            _ => {}
        }
        Ok(Flow::More)
    })?;
    if text.is_empty() && tool_calls.is_empty() {
        let reply = finished.as_ref().map(from_output).unwrap_or_default();
        if !reply.text.is_empty() {
            on_text(&reply.text);
        }
        return Ok(reply);
    }
    Ok(LlmReply { text, tool_calls })
}

fn failure(error: &Value) -> ModelError {
    ModelError::Unavailable(error["message"].as_str().unwrap_or("the response failed").to_owned())
}

fn tool_call(item: &Value) -> ToolCall {
    ToolCall {
        id: item["call_id"].as_str().unwrap_or_default().to_owned(),
        name: item["name"].as_str().unwrap_or_default().to_owned(),
        arguments: item["arguments"].as_str().unwrap_or_default().to_owned(),
    }
}

fn from_output(output: &Value) -> LlmReply {
    let items = output.as_array().map(Vec::as_slice).unwrap_or_default();
    let text = items
        .iter()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    let tool_calls = items.iter().filter(|item| item["type"] == "function_call").map(tool_call).collect();
    LlmReply { text, tool_calls }
}
