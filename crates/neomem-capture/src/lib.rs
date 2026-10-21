use std::fs;
use std::path::Path;

use neomem_core::{
    AgentAdapter, CaptureAgent, CaptureError, Captured, EpochMs, IntakeRecord, Source, Span, Tag,
    redact_secrets,
};
use serde_json::Value;

#[derive(Debug)]
pub struct ClaudeCode;

impl AgentAdapter for ClaudeCode {
    fn agent(&self) -> CaptureAgent {
        CaptureAgent::Cc
    }

    fn read_new(&self, file: &Path, position: u64) -> Result<Captured, CaptureError> {
        read(file, position, self.agent())
    }
}

fn read(file: &Path, position: u64, agent: CaptureAgent) -> Result<Captured, CaptureError> {
    let bytes = fs::read(file).map_err(|error| CaptureError(error.to_string()))?;
    let complete = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |index| index + 1);
    if (complete as u64) < position {
        return Err(CaptureError("transcript shrank below its committed position".to_owned()));
    }
    let mut entries = Vec::new();
    let mut offset = 0_u64;
    for line in bytes[..complete].split_inclusive(|byte| *byte == b'\n') {
        if !line.iter().all(u8::is_ascii_whitespace) {
            let value: Value =
                serde_json::from_slice(line).map_err(|error| CaptureError(error.to_string()))?;
            entries.push((offset, value));
        }
        offset += line.len() as u64;
    }
    let session = entries
        .iter()
        .find_map(|(_, entry)| session_id(entry, agent))
        .ok_or_else(|| CaptureError("transcript has no session ID".to_owned()))?;
    let mut records = Vec::new();
    for (offset, entry) in entries.iter().filter(|(offset, _)| *offset >= position) {
        if let Some(content) = content(entry, agent).filter(|text| !text.trim().is_empty()) {
            let at = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<jiff::Timestamp>().ok())
                .map_or(EpochMs::from_millis(0), |time| EpochMs::from_millis(time.as_millisecond()));
            records.push(IntakeRecord {
                source: Source::new(agent.source()).map_err(|error| CaptureError(error.to_string()))?,
                occurred: Span::instant(at),
                tags: vec![Tag::new("agent-session").map_err(|error| CaptureError(error.to_string()))?],
                content: redact_secrets(&content),
                entity_hints: Vec::new(),
                source_ref: Some(format!("{}#{offset}", file.display())),
                session: Some(session.clone()),
            });
        }
    }
    Ok(Captured { session, position: complete as u64, records })
}

fn session_id(entry: &Value, agent: CaptureAgent) -> Option<String> {
    match agent {
        CaptureAgent::Cc => entry.get("sessionId"),
        CaptureAgent::Codex => entry
            .get("payload")
            .and_then(|payload| payload.get("id"))
            .filter(|_| entry["type"] == "session_meta"),
        CaptureAgent::Pi => entry.get("id").filter(|_| entry["type"] == "session"),
    }
    .and_then(Value::as_str)
    .map(str::to_owned)
}

fn content(entry: &Value, agent: CaptureAgent) -> Option<String> {
    match agent {
        CaptureAgent::Cc if matches!(entry["type"].as_str(), Some("user" | "assistant")) => {
            message(&entry["message"])
        }
        _ => None,
    }
}

fn message(value: &Value) -> Option<String> {
    let role = value["role"].as_str()?;
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    let body = &value["content"];
    let text = if let Some(text) = body.as_str() {
        text.to_owned()
    } else {
        body.as_array()?.iter().filter_map(block).collect::<Vec<_>>().join("\n")
    };
    (!text.is_empty()).then(|| format!("{role}: {text}"))
}

fn block(value: &Value) -> Option<String> {
    match value["type"].as_str()? {
        "text" | "input_text" | "output_text" => value["text"].as_str().map(str::to_owned),
        "tool_use" | "toolCall" => {
            Some(action(value["name"].as_str()?, value.get("input").or_else(|| value.get("arguments"))?))
        }
        "tool_result" => Some(
            if value["is_error"].as_bool() == Some(true) { "action failed" } else { "action completed" }
                .to_owned(),
        ),
        _ => None,
    }
}

fn action(name: &str, arguments: &Value) -> String {
    let detail = ["command", "cmd", "file_path", "path"]
        .iter()
        .find_map(|key| arguments[*key].as_str())
        .unwrap_or_default();
    format!("action {name}: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_passes_the_agent_contract() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agents/skills/neomem-control/fixtures/transcripts/cc/session.jsonl");
        neomem_testkit::agent_contract(&ClaudeCode, &fixture).unwrap();
    }
}
