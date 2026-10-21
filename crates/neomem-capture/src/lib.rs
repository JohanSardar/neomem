mod codex;
mod pi;
pub use codex::Codex;
pub use pi::Pi;

use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
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
    let mut reader = BufReader::new(fs::File::open(file).map_err(|error| CaptureError(error.to_string()))?);
    let length = reader.get_ref().metadata().map_err(|error| CaptureError(error.to_string()))?.len();
    if length < position {
        return Err(CaptureError("transcript shrank below its committed position".to_owned()));
    }
    let session = read_session(&mut reader, agent)?;
    let start = if agent == CaptureAgent::Pi && length > position { 0 } else { position };
    reader.seek(SeekFrom::Start(start)).map_err(|error| CaptureError(error.to_string()))?;
    let (entries, complete) = read_entries(&mut reader, start)?;
    let mut records = Vec::new();
    let selected = if agent == CaptureAgent::Pi {
        pi::active_entries(&entries, position)?
    } else {
        entries.iter().filter(|(offset, _)| *offset >= position).collect()
    };
    for (offset, entry) in selected {
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
    Ok(Captured { session, position: complete, records })
}

type Entries = Vec<(u64, Value)>;

fn read_session(reader: &mut BufReader<fs::File>, agent: CaptureAgent) -> Result<String, CaptureError> {
    loop {
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).map_err(|error| CaptureError(error.to_string()))?;
        if line.last() != Some(&b'\n') {
            return Err(CaptureError("transcript has no complete session header".to_owned()));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let entry: Value = serde_json::from_slice(&line).map_err(|error| CaptureError(error.to_string()))?;
        if let Some(session) = session_id(&entry, agent) {
            return Ok(session);
        }
    }
}

fn read_entries(reader: &mut BufReader<fs::File>, start: u64) -> Result<(Entries, u64), CaptureError> {
    let mut entries = Vec::new();
    let mut position = start;
    loop {
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).map_err(|error| CaptureError(error.to_string()))?;
        if line.last() != Some(&b'\n') {
            break;
        }
        if !line.iter().all(u8::is_ascii_whitespace) {
            entries.push((
                position,
                serde_json::from_slice(&line).map_err(|error| CaptureError(error.to_string()))?,
            ));
        }
        position += line.len() as u64;
    }
    Ok((entries, position))
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
        CaptureAgent::Codex => codex::content(entry),
        CaptureAgent::Pi => pi::content(entry),
        CaptureAgent::Cc => None,
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
        "image" | "input_image" | "image_url" => Some("[image]".to_owned()),
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
    let detail: String = detail.chars().take(512).collect();
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
    #[test]
    fn committed_history_is_not_reparsed() {
        use std::io::Write;
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agents/skills/neomem-control/fixtures/transcripts/cc");
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("session.jsonl");
        let mut bytes = fs::read(fixture.join("session.jsonl")).unwrap();
        fs::write(&file, &bytes).unwrap();
        let first = ClaudeCode.read_new(&file, 0).unwrap();
        let second_line = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        bytes[second_line] = b'!';
        fs::write(&file, bytes).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(&fs::read(fixture.join("tail.json")).unwrap())
            .unwrap();
        let next = ClaudeCode.read_new(&file, first.position).unwrap();
        assert_eq!(next.records.len(), 1);
        assert!(next.records[0].content.contains("Completed after crash"));
    }
    #[test]
    fn codex_passes_the_agent_contract() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agents/skills/neomem-control/fixtures/transcripts/codex/session.jsonl");
        neomem_testkit::agent_contract(&Codex, &fixture).unwrap();
    }
}
