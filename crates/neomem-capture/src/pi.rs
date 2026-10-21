use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use neomem_core::{AgentAdapter, CaptureAgent, CaptureError, Captured};
use serde_json::Value;

#[derive(Debug)]
pub struct Pi;

impl AgentAdapter for Pi {
    fn agent(&self) -> CaptureAgent {
        CaptureAgent::Pi
    }

    fn read_new(&self, file: &Path, position: u64) -> Result<Captured, CaptureError> {
        super::read(file, position, self.agent())
    }
}

pub(super) fn active_entries(
    entries: &super::Entries,
    position: u64,
) -> Result<Vec<&(u64, Value)>, CaptureError> {
    let Some(leaf) = entries.last().filter(|(offset, _)| *offset >= position) else { return Ok(Vec::new()) };
    let by_id: BTreeMap<&str, &(u64, Value)> =
        entries.iter().filter_map(|entry| entry.1["id"].as_str().map(|id| (id, entry))).collect();
    let mut active = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = Some(leaf);
    while let Some(entry) = current {
        if entry.1["type"] == "session" {
            break;
        }
        let id = entry.1["id"].as_str().ok_or_else(|| CaptureError("pi entry has no ID".to_owned()))?;
        if !seen.insert(id) {
            return Err(CaptureError("pi ancestry contains a cycle".to_owned()));
        }
        active.push(entry);
        current = match entry.1["parentId"].as_str() {
            Some(parent) => Some(
                *by_id
                    .get(parent)
                    .ok_or_else(|| CaptureError("pi ancestry has a missing parent".to_owned()))?,
            ),
            None => None,
        };
    }
    active.reverse();
    Ok(active)
}

pub(super) fn content(entry: &Value) -> Option<String> {
    if entry["type"] != "message" {
        return None;
    }
    let message = &entry["message"];
    if message["role"] == "toolResult" {
        let name = message["toolName"].as_str().unwrap_or("tool");
        let call = message["toolCallId"].as_str().unwrap_or_default();
        let status = if message["isError"].as_bool() == Some(true) { "failed" } else { "completed" };
        return Some(format!("action {name} {call} {status}"));
    }
    let text = super::message(message);
    if message["role"] == "assistant" && matches!(message["stopReason"].as_str(), Some("error" | "aborted")) {
        let error: String = message["errorMessage"]
            .as_str()
            .unwrap_or("agent error")
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(512)
            .collect();
        return Some(
            text.map_or_else(
                || format!("assistant: error: {error}"),
                |text| format!("{text}\nerror: {error}"),
            ),
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pi_passes_the_shared_contract_and_ignores_inactive_history() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agents/skills/neomem-control/fixtures/transcripts/pi/session.jsonl");
        neomem_testkit::agent_contract(&Pi, &fixture).unwrap();
        let captured = Pi.read_new(&fixture, 0).unwrap();
        assert_eq!(captured.records.len(), 3);
        assert!(captured.records.iter().all(|record| !record.content.contains("ABANDONED_BRANCH_SENTINEL")
            && !record.content.contains("RAW_OUTPUT_SENTINEL")));
    }

    #[test]
    fn empty_assistant_errors_keep_only_the_summary() {
        let entry = json!({"type":"message","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"command failed\nRAW_OUTPUT_SENTINEL"}});
        let captured = content(&entry).unwrap();
        assert!(captured.contains("command failed"));
        assert!(!captured.contains("RAW_OUTPUT_SENTINEL"));
    }

    #[test]
    fn image_only_user_turns_omit_image_bytes() {
        let entry = json!({"type":"message","message":{"role":"user","content":[{"type":"image","data":"RAW_OUTPUT_SENTINEL","mimeType":"image/png"}]}});
        assert_eq!(content(&entry).as_deref(), Some("user: [image]"));
    }

    #[test]
    fn cyclic_ancestry_is_rejected() {
        let entries = vec![(0, json!({"type":"message","id":"a","parentId":"a"}))];
        assert!(active_entries(&entries, 0).is_err());
    }
}
