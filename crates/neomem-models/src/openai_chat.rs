use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use neomem_core::{Effort, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolCall};
use serde_json::{Value, json};

use crate::http::{self, BodyLines};
use crate::json_schema;
use crate::llm::{self, LlmSettings, Wire};
use crate::openai_responses;
use crate::sse::{self, Flow};

#[derive(Debug)]
pub struct OpenAiChat {
    agent: ureq::Agent,
    url: String,
    key: Option<String>,
    model: String,
    effort: Option<Effort>,
    fast: bool,
    timeout: Duration,
    effort_as_reasoning_object: bool,
}

impl OpenAiChat {
    pub fn new(base_url: &str, key: Option<String>, settings: &LlmSettings) -> Result<Self, ModelError> {
        Ok(Self {
            agent: http::agent(settings.timeout),
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            key,
            model: settings.required_model("openai")?,
            effort: settings.effort,
            fast: settings.fast,
            timeout: settings.timeout,
            effort_as_reasoning_object: base_url.contains("openrouter.ai"),
        })
    }

    fn body(&self, request: &LlmRequest) -> Result<Value, ModelError> {
        let mut messages = Vec::new();
        if !request.system.is_empty() {
            messages.push(json!({ "role": "system", "content": request.system }));
        }
        messages.extend(request.messages.iter().map(chat_message));
        let mut body = json!({ "model": self.model, "messages": messages, "stream": true });
        if request.max_output_tokens > 0 {
            body["max_tokens"] = json!(request.max_output_tokens);
        }
        if !request.tools.is_empty() {
            let tools = request
                .tools
                .iter()
                .map(|tool| {
                    let parameters = llm::tool_parameters(tool)?;
                    Ok(json!({ "type": "function", "function": {
                        "name": tool.name, "description": tool.description, "parameters": parameters } }))
                })
                .collect::<Result<Vec<_>, ModelError>>()?;
            body["tools"] = Value::Array(tools);
        }
        if let Some(schema) = &request.response_schema {
            let schema = llm::parse_schema(schema)?;
            let strict = json_schema::is_strict(&schema);
            body["response_format"] = json!({ "type": "json_schema",
                "json_schema": { "name": "answer", "schema": schema, "strict": strict } });
        }
        if let Some(effort) = self.effort {
            if self.effort_as_reasoning_object {
                body["reasoning"] = json!({ "effort": effort.name() });
            } else {
                body["reasoning_effort"] = json!(effort.name());
            }
        }
        if self.fast {
            body["service_tier"] = json!(openai_responses::PRIORITY_TIER);
        }
        Ok(body)
    }
}

fn chat_message(message: &LlmMessage) -> Value {
    match message {
        LlmMessage::User(text) => json!({ "role": "user", "content": text }),
        LlmMessage::Assistant { text, tool_calls } if tool_calls.is_empty() => {
            json!({ "role": "assistant", "content": text })
        }
        LlmMessage::Assistant { text, tool_calls } => {
            let calls: Vec<Value> = tool_calls
                .iter()
                .map(|call| {
                    json!({ "id": call.id, "type": "function",
                    "function": { "name": call.name, "arguments": call.arguments } })
                })
                .collect();
            let content = if text.is_empty() { Value::Null } else { json!(text) };
            json!({ "role": "assistant", "content": content, "tool_calls": calls })
        }
        LlmMessage::ToolResult { call_id, content } => {
            json!({ "role": "tool", "tool_call_id": call_id, "content": content })
        }
    }
}

impl Wire for OpenAiChat {
    fn has_schema_mode(&self) -> bool {
        true
    }

    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let body = self.body(request)?;
        let headers = http::bearer(self.key.as_deref());
        let (content_type, lines) = http::post_stream(&self.agent, &self.url, &headers, &body, self.timeout)?;
        if content_type.is_some_and(|kind| kind.contains("application/json")) {
            return whole_completion(lines, on_text);
        }
        let mut stream = ChatStream::default();
        sse::read_events(lines, self.timeout, |event| stream.absorb(&event.data, on_text))?;
        Ok(stream.finish())
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("openai:{}", self.model))
    }
}

#[derive(Debug, Default)]
struct ChatStream {
    text: String,
    calls: BTreeMap<u64, ToolCall>,
}

impl ChatStream {
    fn absorb(&mut self, data: &str, on_text: &mut dyn FnMut(&str)) -> Result<Flow, ModelError> {
        if data.trim() == "[DONE]" {
            return Ok(Flow::Done);
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|error| ModelError::InvalidOutput(format!("a stream chunk is not JSON: {error}")))?;
        if let Some(message) = chunk["error"]["message"].as_str() {
            return Err(ModelError::Unavailable(message.to_owned()));
        }
        let delta = &chunk["choices"][0]["delta"];
        if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
            self.text.push_str(text);
            on_text(text);
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let entry = self.calls.entry(call["index"].as_u64().unwrap_or(0)).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = call["id"].as_str().filter(|_| entry.id.is_empty()) {
                id.clone_into(&mut entry.id);
            }
            if let Some(name) = call["function"]["name"].as_str().filter(|_| entry.name.is_empty()) {
                name.clone_into(&mut entry.name);
            }
            entry.arguments.push_str(call["function"]["arguments"].as_str().unwrap_or_default());
        }
        Ok(Flow::More)
    }

    fn finish(self) -> LlmReply {
        LlmReply { text: self.text, tool_calls: self.calls.into_values().collect() }
    }
}

fn whole_completion(mut lines: BodyLines, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
    let mut text = String::new();
    lines.read_to_string(&mut text).map_err(|error| ModelError::Unavailable(error.to_string()))?;
    let completion: Value = serde_json::from_str(&text)
        .map_err(|error| ModelError::InvalidOutput(format!("the completion is not JSON: {error}")))?;
    let message = &completion["choices"][0]["message"];
    let content = message["content"].as_str().unwrap_or_default().to_owned();
    if !content.is_empty() {
        on_text(&content);
    }
    let tool_calls = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|call| ToolCall {
            id: call["id"].as_str().unwrap_or_default().to_owned(),
            name: call["function"]["name"].as_str().unwrap_or_default().to_owned(),
            arguments: call["function"]["arguments"].as_str().unwrap_or_default().to_owned(),
        })
        .collect();
    Ok(LlmReply { text: content, tool_calls })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(base_url: &str) -> OpenAiChat {
        let settings = LlmSettings {
            model: Some("m".to_owned()),
            effort: Some(Effort::Low),
            fast: false,
            timeout: llm::DEFAULT_TIMEOUT,
        };
        OpenAiChat::new(base_url, None, &settings).unwrap()
    }

    fn ping() -> LlmRequest {
        LlmRequest {
            system: String::new(),
            messages: vec![LlmMessage::User("ping".to_owned())],
            response_schema: None,
            tools: Vec::new(),
            max_output_tokens: 0,
        }
    }

    #[test]
    fn openrouter_takes_effort_as_a_reasoning_object_and_others_as_reasoning_effort() {
        let routed = chat("https://openrouter.ai/api/v1").body(&ping()).unwrap();
        assert_eq!(routed["reasoning"]["effort"], "low");
        assert!(routed["reasoning_effort"].is_null());
        let direct = chat("http://127.0.0.1:11434/v1").body(&ping()).unwrap();
        assert_eq!(direct["reasoning_effort"], "low");
        assert!(direct["max_tokens"].is_null(), "no limit is sent when the request sets none");
        assert!(direct["service_tier"].is_null());
    }

    #[test]
    fn a_fast_role_asks_for_the_priority_tier_at_max_effort() {
        let settings = LlmSettings {
            model: Some("m".to_owned()),
            effort: Some(Effort::Max),
            fast: true,
            timeout: llm::DEFAULT_TIMEOUT,
        };
        let body =
            OpenAiChat::new("http://127.0.0.1:11434/v1", None, &settings).unwrap().body(&ping()).unwrap();
        assert_eq!(
            (body["reasoning_effort"].as_str(), body["service_tier"].as_str()),
            (Some("max"), Some("priority"))
        );
        let responses = openai_responses::body("m", Some(Effort::Xhigh), true, &ping()).unwrap();
        assert_eq!(responses["reasoning"]["effort"], "xhigh");
        assert_eq!(responses["service_tier"], "priority");
    }
}
