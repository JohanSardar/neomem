use std::collections::BTreeMap;
use std::time::Duration;

use neomem_core::{Effort, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolCall};
use serde_json::{Value, json};

use crate::http;
use crate::llm::{self, LlmSettings, Wire};
use crate::sse::{self, Flow};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 16_000;

#[derive(Debug)]
pub struct AnthropicLlm {
    agent: ureq::Agent,
    url: String,
    key: String,
    model: String,
    effort: Option<Effort>,
    timeout: Duration,
}

impl AnthropicLlm {
    pub fn new(base_url: &str, key: String, settings: &LlmSettings) -> Result<Self, ModelError> {
        Ok(Self {
            agent: http::agent(settings.timeout),
            url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
            key,
            model: settings.required_model("anthropic")?,
            effort: settings.effort,
            timeout: settings.timeout,
        })
    }

    fn body(&self, request: &LlmRequest) -> Result<Value, ModelError> {
        let max_tokens =
            if request.max_output_tokens > 0 { request.max_output_tokens } else { DEFAULT_MAX_TOKENS };
        let mut body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "stream": true,
            "messages": messages(request),
        });
        if !request.system.is_empty() {
            body["system"] =
                json!([{ "type": "text", "text": request.system, "cache_control": { "type": "ephemeral" } }]);
        }
        if !request.tools.is_empty() {
            let tools = request
                .tools
                .iter()
                .map(|tool| {
                    Ok(json!({ "name": tool.name, "description": tool.description,
                        "input_schema": llm::tool_parameters(tool)? }))
                })
                .collect::<Result<Vec<_>, ModelError>>()?;
            body["tools"] = Value::Array(tools);
        }
        let mut output = serde_json::Map::new();
        if let Some(effort) = self.effort {
            output.insert("effort".to_owned(), json!(effort.name()));
        }
        if let Some(schema) = &request.response_schema {
            output.insert(
                "format".to_owned(),
                json!({ "type": "json_schema", "schema": llm::parse_schema(schema)? }),
            );
        }
        if !output.is_empty() {
            body["output_config"] = Value::Object(output);
        }
        Ok(body)
    }
}

fn messages(request: &LlmRequest) -> Vec<Value> {
    let mut turns: Vec<(&str, Vec<Value>)> = Vec::new();
    for message in &request.messages {
        let (role, blocks) = match message {
            LlmMessage::User(text) => ("user", vec![json!({ "type": "text", "text": text })]),
            LlmMessage::Assistant { text, tool_calls } => {
                let spoken = (!text.is_empty()).then(|| json!({ "type": "text", "text": text }));
                let calls = tool_calls.iter().map(|call| {
                    let input: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                    json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": input })
                });
                ("assistant", spoken.into_iter().chain(calls).collect())
            }
            LlmMessage::ToolResult { call_id, content } => {
                ("user", vec![json!({ "type": "tool_result", "tool_use_id": call_id, "content": content })])
            }
        };
        match turns.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => last_blocks.extend(blocks),
            _ if blocks.is_empty() => {}
            _ => turns.push((role, blocks)),
        }
    }
    turns.into_iter().map(|(role, blocks)| json!({ "role": role, "content": blocks })).collect()
}

impl Wire for AnthropicLlm {
    fn has_schema_mode(&self) -> bool {
        true
    }

    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let body = self.body(request)?;
        let headers = [("x-api-key", self.key.clone()), ("anthropic-version", API_VERSION.to_owned())];
        let (_, lines) = http::post_stream(&self.agent, &self.url, &headers, &body, self.timeout)?;
        let mut stream = MessageStream::default();
        sse::read_events(lines, self.timeout, |event| stream.absorb(&event.data, on_text))?;
        Ok(stream.finish())
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("anthropic:{}", self.model))
    }
}

#[derive(Debug, Default)]
struct MessageStream {
    text: String,
    tool_uses: BTreeMap<u64, (ToolCall, String)>,
}

impl MessageStream {
    fn absorb(&mut self, data: &str, on_text: &mut dyn FnMut(&str)) -> Result<Flow, ModelError> {
        let event: Value = serde_json::from_str(data)
            .map_err(|error| ModelError::InvalidOutput(format!("a stream event is not JSON: {error}")))?;
        let index = event["index"].as_u64().unwrap_or(0);
        match event["type"].as_str().unwrap_or_default() {
            "content_block_start" if event["content_block"]["type"] == "tool_use" => {
                let block = &event["content_block"];
                let call = ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_owned(),
                    name: block["name"].as_str().unwrap_or_default().to_owned(),
                    arguments: String::new(),
                };
                let given = block["input"].as_object().filter(|input| !input.is_empty());
                self.tool_uses.insert(
                    index,
                    (call, given.map(|input| Value::from(input.clone()).to_string()).unwrap_or_default()),
                );
            }
            "content_block_delta" => match event["delta"]["type"].as_str().unwrap_or_default() {
                "text_delta" => {
                    let text = event["delta"]["text"].as_str().unwrap_or_default();
                    self.text.push_str(text);
                    on_text(text);
                }
                "input_json_delta" => {
                    if let Some((_, json)) = self.tool_uses.get_mut(&index) {
                        json.push_str(event["delta"]["partial_json"].as_str().unwrap_or_default());
                    }
                }
                _ => {}
            },
            "message_stop" => return Ok(Flow::Done),
            "error" => {
                let message = event["error"]["message"].as_str().unwrap_or("the stream reported an error");
                return Err(ModelError::Unavailable(message.to_owned()));
            }
            _ => {}
        }
        Ok(Flow::More)
    }

    fn finish(self) -> LlmReply {
        let tool_calls = self
            .tool_uses
            .into_values()
            .map(|(mut call, json)| {
                call.arguments = if json.trim().is_empty() { "{}".to_owned() } else { json };
                call
            })
            .collect();
        LlmReply { text: self.text, tool_calls }
    }
}
