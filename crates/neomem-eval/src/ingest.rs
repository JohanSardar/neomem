use crate::{
    daemon::Daemon,
    data::{self, BenchmarkItem},
};
use anyhow::{Context, ensure};
use neomem_proto::{Request, Response};
use serde_json::json;
use std::collections::BTreeMap;
pub fn replay(daemon: &Daemon, question: &BenchmarkItem) -> anyhow::Result<BTreeMap<String, String>> {
    let mut evidence = BTreeMap::new();
    for (index, turns) in question.haystack_sessions.iter().enumerate() {
        for turn in turns {
            let record = json!({"source":"eval","occurred":{"start":data::date(&question.haystack_dates[index])?,"end":data::date(&question.haystack_dates[index])?},
                    "tags":[],"content":format!("{}: {}",turn.role,turn.content),"entity_hints":[],
                    "source_ref":null,"session":question.haystack_session_ids[index]});
            let request = serde_json::from_value(json!({"command":"add","arguments":record}))?;
            let Response::Added(receipt) = daemon.call(request)? else {
                anyhow::bail!("add did not return receipt");
            };
            if question.answer_session_ids.contains(&question.haystack_session_ids[index]) {
                evidence.insert(receipt.id.to_string(), question.haystack_session_ids[index].clone());
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for capture in &question.captures {
        ensure!(["cc", "codex"].contains(&capture.agent.as_str()), "unsupported capture agent");
        ensure!(!capture.session.contains(['/', '\\']), "capture session must be a file name");
        let path = daemon
            .home
            .root()
            .join("capture")
            .join(&capture.agent)
            .join(format!("{}.jsonl", capture.session));
        std::fs::create_dir_all(path.parent().context("capture directory")?)?;
        std::fs::copy(&capture.path, &path)?;
        daemon.call(serde_json::from_value(json!({"command":"capture","arguments":{"agent":capture.agent,"session":capture.session,"transcript_path":path,"event":"end"}}))?)?;
        let Response::RawHits(records) = daemon.call(Request::Raw(neomem_proto::RawQuery {
            text: String::new(),
            limit: 200,
            from: None,
            to: None,
            source: None,
        }))?
        else {
            anyhow::bail!("missing captured intake");
        };
        ensure!(records.len() < 200, "capture evidence exceeds raw page; use bounded transcript windows");
        for record in records {
            if seen.insert(record.id.to_string()) && question.answer_session_ids.contains(&capture.session) {
                evidence.insert(record.id.to_string(), capture.session.clone());
            }
        }
    }
    Ok(evidence)
}
