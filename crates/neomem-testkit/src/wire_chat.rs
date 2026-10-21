use neomem_core::{LlmMessage, LlmRequest, ToolCall, ToolSpec};
use serde_json::{Value, json};

use crate::transport::{Reply, TimedEvent};
use crate::{ScriptedAnswer, ScriptedLlm};

pub fn reply(llm: &ScriptedLlm, body: &Value) -> Reply {
    let answer = llm.answer(&request(body));
    if body["stream"] == true { Reply::Events(events(&answer)) } else { Reply::ok(completion(&answer, body)) }
}

pub fn request(body: &Value) -> LlmRequest {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for message in body["messages"].as_array().into_iter().flatten() {
        let text = content_text(&message["content"]);
        match message["role"].as_str() {
            Some("system" | "developer") => system.push(text),
            Some("assistant") => {
                messages.push(LlmMessage::Assistant { text, tool_calls: tool_calls(&message["tool_calls"]) });
            }
            Some("tool") => messages.push(LlmMessage::ToolResult {
                call_id: message["tool_call_id"].as_str().unwrap_or_default().to_owned(),
                content: text,
            }),
            _ => messages.push(LlmMessage::User(text)),
        }
    }
    let schema = &body["response_format"]["json_schema"]["schema"];
    LlmRequest {
        system: system.join("\n"),
        messages,
        response_schema: (!schema.is_null()).then(|| schema.to_string()),
        tools: body["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|tool| tool_spec(&tool["function"]))
            .collect(),
        max_output_tokens: body["max_tokens"]
            .as_u64()
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or(0),
    }
}

pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("")
        }
        _ => String::new(),
    }
}

pub fn tool_spec(function: &Value) -> ToolSpec {
    ToolSpec {
        name: function["name"].as_str().unwrap_or_default().to_owned(),
        description: function["description"].as_str().unwrap_or_default().to_owned(),
        parameters_schema: function["parameters"].to_string(),
    }
}

fn tool_calls(calls: &Value) -> Vec<ToolCall> {
    calls
        .as_array()
        .into_iter()
        .flatten()
        .map(|call| ToolCall {
            id: call["id"].as_str().unwrap_or_default().to_owned(),
            name: call["function"]["name"].as_str().unwrap_or_default().to_owned(),
            arguments: call["function"]["arguments"].as_str().unwrap_or_default().to_owned(),
        })
        .collect()
}

pub fn split_in_two(text: &str) -> (&str, &str) {
    let middle = (0..=text.len() / 2).rev().find(|index| text.is_char_boundary(*index)).unwrap_or(0);
    text.split_at(middle)
}

fn chunk(delta: &Value, finish_reason: Option<&str>) -> Value {
    json!({ "id": "chatcmpl-fake", "object": "chat.completion.chunk",
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish_reason }] })
}

fn events(answer: &ScriptedAnswer) -> Vec<TimedEvent> {
    let mut wait = answer.delay_ms;
    let mut events = Vec::new();
    for delta in &answer.deltas {
        events.push(TimedEvent::data(wait, &chunk(&json!({ "content": delta }), None)));
        wait = answer.delta_delay_ms;
    }
    for (index, call) in answer.tool_calls.iter().enumerate() {
        let (head, tail) = split_in_two(&call.arguments);
        let opening = json!({ "tool_calls": [{ "index": index, "id": call.id, "type": "function",
            "function": { "name": call.name, "arguments": head } }] });
        let rest = json!({ "tool_calls": [{ "index": index, "function": { "arguments": tail } }] });
        events.push(TimedEvent::data(wait, &chunk(&opening, None)));
        events.push(TimedEvent::data(0, &chunk(&rest, None)));
        wait = 0;
    }
    let finish = if answer.tool_calls.is_empty() { "stop" } else { "tool_calls" };
    events.push(TimedEvent::data(wait, &chunk(&json!({}), Some(finish))));
    events.push(TimedEvent::done());
    events
}

fn completion(answer: &ScriptedAnswer, body: &Value) -> Value {
    std::thread::sleep(std::time::Duration::from_millis(answer.delay_ms));
    let mut message = json!({ "role": "assistant", "content": answer.text() });
    if !answer.tool_calls.is_empty() {
        message["tool_calls"] = answer
            .tool_calls
            .iter()
            .map(|call| {
                json!({ "id": call.id, "type": "function",
                "function": { "name": call.name, "arguments": call.arguments } })
            })
            .collect();
    }
    json!({ "id": "chatcmpl-fake", "object": "chat.completion", "model": body["model"],
        "choices": [{ "index": 0, "message": message, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 12, "completion_tokens": answer.deltas.len() } })
}
