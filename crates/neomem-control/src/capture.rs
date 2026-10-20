use std::fs;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use neomem_core::{CaptureAgent, CaptureEvent, CaptureSignal};
use neomem_proto::{Client, Request, Response};

use crate::binaries::Binaries;
use crate::session;

pub fn hook(agent: &str, event: &str, payload: Option<&Path>) -> anyhow::Result<bool> {
    let home = session::resolve_home(None)?;
    let binaries = Binaries::beside_this_executable()?;
    let body = payload.map_or_else(|| Ok("{}".to_owned()), fs::read_to_string)?;
    let mut child = Command::new(binaries.neomem)
        .args(["hook", agent, event])
        .env("NEOMEM_HOME", home.root())
        .stdin(Stdio::piped())
        .spawn()?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(body.as_bytes())?;
    }
    Ok(child.wait()?.success())
}

pub fn replay(transcript: &Path, agent: Option<&str>) -> anyhow::Result<bool> {
    let home = session::resolve_home(None)?;
    let agent = if let Some(agent) = agent {
        serde_json::from_value(serde_json::Value::String(agent.to_owned()))?
    } else {
        let first = std::io::BufReader::new(fs::File::open(transcript)?)
            .lines()
            .next()
            .transpose()?
            .unwrap_or_default();
        let value: serde_json::Value = serde_json::from_str(&first)?;
        match value["type"].as_str() {
            Some("session_meta") => CaptureAgent::Codex,
            Some("session") => CaptureAgent::Pi,
            _ => CaptureAgent::Cc,
        }
    };
    let request = Request::Capture(CaptureSignal {
        agent,
        session: String::new(),
        transcript_path: Some(fs::canonicalize(transcript)?),
        event: CaptureEvent::Stop,
    });
    let response = Client::connect(&home, Duration::from_secs(30))?.call(request)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(matches!(response, Response::Captured))
}
