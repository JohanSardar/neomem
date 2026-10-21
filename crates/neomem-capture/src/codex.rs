use std::path::Path;

use neomem_core::{AgentAdapter, CaptureAgent, CaptureError, Captured};
use serde_json::Value;

#[derive(Debug)]
pub struct Codex;

impl AgentAdapter for Codex {
    fn agent(&self) -> CaptureAgent {
        CaptureAgent::Codex
    }

    fn read_new(&self, file: &Path, position: u64) -> Result<Captured, CaptureError> {
        super::read(file, position, self.agent())
    }
}

pub(super) fn content(entry: &Value) -> Option<String> {
    let payload = &entry["payload"];
    if entry["type"] == "event_msg" && payload["type"] == "error" {
        return Some(format!("error: {}", compact(payload["message"].as_str().unwrap_or("agent error"))));
    }
    if entry["type"] != "response_item" {
        return None;
    }
    match payload["type"].as_str()? {
        "message" if is_injected_context(payload) => None,
        "message" => super::message(payload),
        "function_call" => {
            let arguments = match &payload["arguments"] {
                Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                value => value.clone(),
            };
            Some(super::action(payload["name"].as_str()?, &arguments))
        }
        "custom_tool_call" => {
            let name = payload["name"].as_str()?;
            let paths = payload["input"]
                .as_str()
                .unwrap_or_default()
                .lines()
                .filter_map(|line| {
                    ["*** Update File: ", "*** Add File: ", "*** Delete File: ", "*** Move to: "]
                        .iter()
                        .find_map(|prefix| line.strip_prefix(prefix))
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("action {name}: {}", compact(&paths)))
        }
        "function_call_output" | "custom_tool_call_output" => Some(result(payload)),
        "web_search_call" => {
            let action = &payload["action"];
            let query = action["query"].as_str().map_or_else(
                || {
                    action["queries"]
                        .as_array()
                        .map(|queries| {
                            queries.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                        })
                        .unwrap_or_default()
                },
                str::to_owned,
            );
            Some(format!("action web_search: {}", compact(&query)))
        }
        "local_shell_call" => {
            let command = payload["action"]["command"]
                .as_array()
                .map(|parts| parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            Some(format!("action shell: {}", compact(&command)))
        }
        "image_generation_call" => Some("action image_generation".to_owned()),
        _ => None,
    }
}

fn result(payload: &Value) -> String {
    let output = &payload["output"];
    let structured: Option<Value> = output.as_str().and_then(|text| serde_json::from_str(text).ok());
    let status = structured.as_ref().unwrap_or(output);
    let code =
        status["exit_code"].as_i64().or_else(|| status["metadata"]["exit_code"].as_i64()).or_else(|| {
            output
                .as_str()?
                .lines()
                .find_map(|line| line.strip_prefix("Process exited with code ")?.trim().parse::<i64>().ok())
        });
    let status = if payload["is_error"].as_bool() == Some(true) || code.is_some_and(|code| code != 0) {
        "failed"
    } else {
        "completed"
    };
    let call = payload["call_id"].as_str().unwrap_or_default();
    code.map_or_else(
        || format!("action {call} {status}"),
        |code| format!("action {call} {status}, exit {code}"),
    )
}

const INJECTED_CONTEXT: &[&str] = &["<environment_context>", "<user_instructions>"];

fn is_injected_context(payload: &Value) -> bool {
    payload["role"] == "user"
        && payload["content"]
            .as_array()
            .and_then(|blocks| blocks.first())
            .and_then(|block| block["text"].as_str())
            .is_some_and(|text| INJECTED_CONTEXT.iter().any(|marker| text.trim_start().starts_with(marker)))
}

fn compact(text: &str) -> String {
    text.lines().next().unwrap_or_default().chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_actions_keep_paths_and_omit_file_bodies() {
        let entry = json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/main.rs\n@@\n+RAW_OUTPUT_SENTINEL\n*** End Patch"}});
        let captured = content(&entry).unwrap();
        assert!(captured.contains("src/main.rs"));
        assert!(!captured.contains("RAW_OUTPUT_SENTINEL"));
    }

    #[test]
    fn structured_tool_failures_keep_exit_status_and_omit_output() {
        let output = json!({"output":"RAW_OUTPUT_SENTINEL","metadata":{"exit_code":1}}).to_string();
        let entry = json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"test","output":output}});
        let captured = content(&entry).unwrap();
        assert!(captured.contains("failed"));
        assert!(captured.contains("exit 1"));
        assert!(!captured.contains("RAW_OUTPUT_SENTINEL"));
    }

    #[test]
    fn context_codex_injects_as_a_user_message_is_not_a_turn() {
        let injected = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/work</cwd>\n</environment_context>"}]}});
        let typed = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"What is the Edge ship date?"}]}});
        assert_eq!(content(&injected), None);
        assert_eq!(content(&typed).as_deref(), Some("user: What is the Edge ship date?"));
    }

    #[test]
    fn errors_keep_a_compact_summary() {
        let entry = json!({"type":"event_msg","payload":{"type":"error","message":"execution failed\nRAW_OUTPUT_SENTINEL"}});
        let captured = content(&entry).unwrap();
        assert!(captured.contains("execution failed"));
        assert!(!captured.contains("RAW_OUTPUT_SENTINEL"));
    }
}
