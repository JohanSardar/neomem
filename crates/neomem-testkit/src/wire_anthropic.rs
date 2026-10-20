use neomem_core::{LlmMessage, LlmRequest, ToolCall, ToolSpec};
use serde_json::{Value, json};

use crate::transport::{Incoming, Reply, TimedEvent};
use crate::wire_chat::{content_text, split_in_two};
use crate::{ScriptedAnswer, ScriptedLlm};

pub fn reply(llm: &ScriptedLlm, incoming: &Incoming) -> Reply {
    if incoming.header("x-api-key").is_none_or(str::is_empty)
        || incoming.header("anthropic-version").is_none()
    {
        return Reply::refused(401, "x-api-key and anthropic-version are required");
    }
    let body = &incoming.body;
    let answer = llm.answer(&request(body));
    let thinks = !body["output_config"]["effort"].is_null();
    if body["stream"] == true {
        Reply::Events(events(&answer, thinks))
    } else {
        std::thread::sleep(std::time::Duration::from_millis(answer.delay_ms));
        Reply::ok(message(&answer, &content(&answer)))
    }
}

pub fn request(body: &Value) -> LlmRequest {
    let mut messages = Vec::new();
    for message in body["messages"].as_array().into_iter().flatten() {
        let blocks = match &message["content"] {
            Value::String(text) => vec![json!({ "type": "text", "text": text })],
            Value::Array(blocks) => blocks.clone(),
            _ => Vec::new(),
        };
        if message["role"] == "assistant" {
            messages.push(assistant(&blocks));
            continue;
        }
        for block in &blocks {
            if block["type"] == "tool_result" {
                messages.push(LlmMessage::ToolResult {
                    call_id: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                    content: content_text(&block["content"]),
                });
            } else if let Some(text) = block["text"].as_str() {
                messages.push(LlmMessage::User(text.to_owned()));
            }
        }
    }
    let schema = &body["output_config"]["format"]["schema"];
    LlmRequest {
        system: content_text(&body["system"]),
        messages,
        response_schema: (!schema.is_null()).then(|| schema.to_string()),
        tools: body["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|tool| ToolSpec {
                name: tool["name"].as_str().unwrap_or_default().to_owned(),
                description: tool["description"].as_str().unwrap_or_default().to_owned(),
                parameters_schema: tool["input_schema"].to_string(),
            })
            .collect(),
        max_output_tokens: body["max_tokens"]
            .as_u64()
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or(0),
    }
}

fn assistant(blocks: &[Value]) -> LlmMessage {
    let text =
        blocks.iter().filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str());
    let tool_calls = blocks
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| ToolCall {
            id: block["id"].as_str().unwrap_or_default().to_owned(),
            name: block["name"].as_str().unwrap_or_default().to_owned(),
            arguments: block["input"].to_string(),
        })
        .collect();
    LlmMessage::Assistant { text: text.collect::<Vec<_>>().join(""), tool_calls }
}

fn content(answer: &ScriptedAnswer) -> Vec<Value> {
    let mut blocks = Vec::new();
    if !answer.deltas.is_empty() {
        blocks.push(json!({ "type": "text", "text": answer.text() }));
    }
    for call in &answer.tool_calls {
        let input: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
        blocks.push(json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": input }));
    }
    blocks
}

fn message(answer: &ScriptedAnswer, content: &[Value]) -> Value {
    let stop = if answer.tool_calls.is_empty() { "end_turn" } else { "tool_use" };
    json!({ "id": "msg_fake", "type": "message", "role": "assistant", "model": "claude-fake",
        "content": content, "stop_reason": stop, "usage": { "input_tokens": 12, "output_tokens": answer.deltas.len() } })
}

fn events(answer: &ScriptedAnswer, thinks: bool) -> Vec<TimedEvent> {
    let start = json!({ "type": "message_start", "message": message(&ScriptedAnswer::default(), &[]) });
    let mut events = vec![TimedEvent::named(answer.delay_ms, "message_start", &start)];
    events.push(TimedEvent::named(0, "ping", &json!({ "type": "ping" })));
    let mut index = 0;
    let mut block = |events: &mut Vec<TimedEvent>, opening: Value, deltas: Vec<(u64, Value)>| {
        events.push(TimedEvent::named(
            0,
            "content_block_start",
            &json!({ "type": "content_block_start", "index": index, "content_block": opening }),
        ));
        for (wait, delta) in deltas {
            events.push(TimedEvent::named(
                wait,
                "content_block_delta",
                &json!({ "type": "content_block_delta", "index": index, "delta": delta }),
            ));
        }
        events.push(TimedEvent::named(
            0,
            "content_block_stop",
            &json!({ "type": "content_block_stop", "index": index }),
        ));
        index += 1;
    };
    if thinks {
        let thinking = vec![(0, json!({ "type": "thinking_delta", "thinking": "Thinking it over." }))];
        block(&mut events, json!({ "type": "thinking", "thinking": "" }), thinking);
    }
    if !answer.deltas.is_empty() {
        let deltas = answer
            .deltas
            .iter()
            .enumerate()
            .map(|(at, text)| {
                (
                    if at == 0 { 0 } else { answer.delta_delay_ms },
                    json!({ "type": "text_delta", "text": text }),
                )
            })
            .collect();
        block(&mut events, json!({ "type": "text", "text": "" }), deltas);
    }
    for call in &answer.tool_calls {
        let (head, tail) = split_in_two(&call.arguments);
        let deltas = [head, tail]
            .iter()
            .map(|part| (0, json!({ "type": "input_json_delta", "partial_json": part })))
            .collect();
        block(
            &mut events,
            json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": {} }),
            deltas,
        );
    }
    let stop = if answer.tool_calls.is_empty() { "end_turn" } else { "tool_use" };
    let delta = json!({ "type": "message_delta", "delta": { "stop_reason": stop }, "usage": { "output_tokens": answer.deltas.len() } });
    events.push(TimedEvent::named(0, "message_delta", &delta));
    events.push(TimedEvent::named(0, "message_stop", &json!({ "type": "message_stop" })));
    events
}
