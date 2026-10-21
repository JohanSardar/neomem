use neomem_core::{LlmMessage, LlmRequest, ToolCall};
use serde_json::{Value, json};

use crate::transport::{Reply, TimedEvent};
use crate::wire_chat::{content_text, tool_spec};
use crate::{ScriptedAnswer, ScriptedLlm};

pub fn reply(llm: &ScriptedLlm, body: &Value) -> Reply {
    let answer = llm.answer(&request(body));
    if body["stream"] == true {
        Reply::Events(events(&answer))
    } else {
        std::thread::sleep(std::time::Duration::from_millis(answer.delay_ms));
        Reply::ok(response(&answer, "completed"))
    }
}

pub fn request(body: &Value) -> LlmRequest {
    let mut system: Vec<String> = body["instructions"].as_str().map(str::to_owned).into_iter().collect();
    let mut messages: Vec<LlmMessage> = Vec::new();
    let items = match &body["input"] {
        Value::String(text) => vec![json!({ "role": "user", "content": text })],
        Value::Array(items) => items.clone(),
        _ => Vec::new(),
    };
    for item in &items {
        match (item["type"].as_str(), item["role"].as_str()) {
            (Some("function_call"), _) => {
                let call = ToolCall {
                    id: item["call_id"].as_str().unwrap_or_default().to_owned(),
                    name: item["name"].as_str().unwrap_or_default().to_owned(),
                    arguments: item["arguments"].as_str().unwrap_or_default().to_owned(),
                };
                match messages.last_mut() {
                    Some(LlmMessage::Assistant { tool_calls, .. }) => tool_calls.push(call),
                    _ => messages.push(LlmMessage::Assistant { text: String::new(), tool_calls: vec![call] }),
                }
            }
            (Some("function_call_output"), _) => messages.push(LlmMessage::ToolResult {
                call_id: item["call_id"].as_str().unwrap_or_default().to_owned(),
                content: item["output"].as_str().unwrap_or_default().to_owned(),
            }),
            (_, Some("system" | "developer")) => system.push(content_text(&item["content"])),
            (_, Some("assistant")) => messages
                .push(LlmMessage::Assistant { text: content_text(&item["content"]), tool_calls: Vec::new() }),
            _ => messages.push(LlmMessage::User(content_text(&item["content"]))),
        }
    }
    let schema = &body["text"]["format"]["schema"];
    LlmRequest {
        system: system.join("\n"),
        messages,
        response_schema: (!schema.is_null()).then(|| schema.to_string()),
        tools: body["tools"].as_array().into_iter().flatten().map(tool_spec).collect(),
        max_output_tokens: body["max_output_tokens"]
            .as_u64()
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or(0),
    }
}

fn message(answer: &ScriptedAnswer, status: &str) -> Value {
    let content = if status == "completed" {
        vec![json!({ "type": "output_text", "text": answer.text(), "annotations": [] })]
    } else {
        Vec::new()
    };
    json!({ "type": "message", "id": "msg_fake", "role": "assistant", "status": status, "content": content })
}

fn output(answer: &ScriptedAnswer) -> Vec<Value> {
    let mut output = Vec::new();
    if !answer.deltas.is_empty() {
        output.push(message(answer, "completed"));
    }
    output.extend(answer.tool_calls.iter().map(function_call));
    output
}

fn function_call(call: &ToolCall) -> Value {
    json!({ "type": "function_call", "id": format!("fc_{}", call.id), "call_id": call.id,
        "name": call.name, "arguments": call.arguments, "status": "completed" })
}

fn response(answer: &ScriptedAnswer, status: &str) -> Value {
    let output_tokens = answer.deltas.len();
    json!({ "id": "resp_fake", "object": "response", "status": status, "output": output(answer),
        "usage": { "input_tokens": 12, "input_tokens_details": { "cached_tokens": 0 },
            "output_tokens": output_tokens, "output_tokens_details": { "reasoning_tokens": 0 },
            "total_tokens": 12 + output_tokens } })
}

fn event(wait: u64, data: &Value) -> TimedEvent {
    TimedEvent::named(wait, data["type"].as_str().unwrap_or_default(), data)
}

fn text_events(answer: &ScriptedAnswer, events: &mut Vec<TimedEvent>) {
    let part = json!({ "type": "output_text", "text": "", "annotations": [] });
    events.push(event(
        0,
        &json!({ "type": "response.output_item.added", "output_index": 0,
        "item": message(answer, "in_progress") }),
    ));
    events.push(event(
        0,
        &json!({ "type": "response.content_part.added", "item_id": "msg_fake",
        "output_index": 0, "content_index": 0, "part": part }),
    ));
    let mut wait = 0;
    for delta in &answer.deltas {
        events.push(event(
            wait,
            &json!({ "type": "response.output_text.delta", "item_id": "msg_fake",
            "output_index": 0, "content_index": 0, "delta": delta }),
        ));
        wait = answer.delta_delay_ms;
    }
    events.push(event(
        0,
        &json!({ "type": "response.output_text.done", "item_id": "msg_fake",
        "output_index": 0, "content_index": 0, "text": answer.text() }),
    ));
    events.push(event(
        0,
        &json!({ "type": "response.content_part.done", "item_id": "msg_fake",
        "output_index": 0, "content_index": 0,
        "part": { "type": "output_text", "text": answer.text(), "annotations": [] } }),
    ));
    events.push(event(
        0,
        &json!({ "type": "response.output_item.done", "output_index": 0,
        "item": message(answer, "completed") }),
    ));
}

fn events(answer: &ScriptedAnswer) -> Vec<TimedEvent> {
    let created = json!({ "type": "response.created", "response": response(&ScriptedAnswer::default(), "in_progress") });
    let mut events = vec![TimedEvent::named(answer.delay_ms, "response.created", &created)];
    let first_call_index = usize::from(!answer.deltas.is_empty());
    if !answer.deltas.is_empty() {
        text_events(answer, &mut events);
    }
    for (index, call) in answer.tool_calls.iter().enumerate() {
        let output_index = first_call_index + index;
        events.push(event(
            0,
            &json!({ "type": "response.output_item.added", "output_index": output_index,
            "item": function_call(call) }),
        ));
        events.push(event(
            0,
            &json!({ "type": "response.output_item.done", "output_index": output_index,
            "item": function_call(call) }),
        ));
    }
    let completed = json!({ "type": "response.completed", "response": response(answer, "completed") });
    events.push(TimedEvent::named(0, "response.completed", &completed));
    events
}
