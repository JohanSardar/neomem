use std::fmt::Write;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{CaptureAgent, CaptureEvent, CaptureSignal};
use neomem_proto::{
    Envelope, Home, RecallMemory, RecallQuery, Request, Response, read_frame, spool, write_frame,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct HookPayload {
    #[serde(default, alias = "sessionId", alias = "thread_id")]
    session_id: String,
    #[serde(default, alias = "session_file")]
    transcript_path: Option<std::path::PathBuf>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    turn_id: Option<String>,
}

enum Progress {
    Recalling,
    Complete,
}

pub fn run(arguments: &[String]) {
    let started = Instant::now();
    let arguments = arguments.to_vec();
    let (sender, receiver) = mpsc::channel();
    let _ = thread::Builder::new().spawn(move || {
        let mut body = String::new();
        let output = std::io::stdin()
            .take(4_194_305)
            .read_to_string(&mut body)
            .ok()
            .and_then(|_| send(&arguments, &body, started, &sender).ok().flatten());
        if let Some(output) = output
            && let Ok(descriptor) = std::io::stdout().as_fd().try_clone_to_owned()
        {
            let mut stdout = std::fs::File::from(descriptor);
            let _ = std::io::Write::write_all(&mut stdout, output.as_bytes());
        }
        let _ = sender.send(Progress::Complete);
    });
    let mut deadline = started + Duration::from_millis(20);
    while let Ok(Progress::Recalling) =
        receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        deadline = started + Duration::from_millis(3500);
    }
}

fn send(
    arguments: &[String],
    body: &str,
    started: Instant,
    sender: &mpsc::Sender<Progress>,
) -> anyhow::Result<Option<String>> {
    let agent: CaptureAgent =
        serde_json::from_value(Value::String(arguments.first().cloned().unwrap_or_default()))?;
    let event: CaptureEvent =
        serde_json::from_value(Value::String(arguments.get(1).cloned().unwrap_or_default()))?;
    let payload: HookPayload = serde_json::from_str(body)?;
    let signal = CaptureSignal {
        agent,
        session: payload.session_id.clone(),
        transcript_path: payload.transcript_path,
        event,
    };
    let home = Home::from_env().ok_or_else(|| anyhow::anyhow!("missing home"))?;
    let envelope = Envelope::current(Request::Capture(signal));
    if started.elapsed() >= Duration::from_millis(30) {
        return Ok(None);
    }
    let stream = capture(&home, &envelope)?;
    if event != CaptureEvent::Prompt || agent == CaptureAgent::Pi {
        return Ok(None);
    }
    let config: toml::Table =
        toml::from_str(&std::fs::read_to_string(home.config_path()).unwrap_or_default())?;
    let project = payload.cwd.as_deref().map(crate::recall::project_key).transpose()?;
    let name = arguments.first().map_or("", String::as_str);
    if !crate::recall::enabled(&config, name, project.as_deref()) {
        return Ok(None);
    }
    let _ = sender.send(Progress::Recalling);
    let Some(mut stream) = stream else { return Ok(None) };
    let remaining = Duration::from_millis(3500).saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Ok(None);
    }
    stream.set_read_timeout(Some(remaining))?;
    stream.set_write_timeout(Some(remaining))?;
    if !matches!(read_frame::<Response>(&mut stream)?, Response::Captured) {
        return Ok(None);
    }
    write_frame(
        &mut stream,
        &Envelope::current(Request::Recall(RecallQuery {
            turn: payload.turn_id,
            agent: name.to_owned(),
            session: payload.session_id,
            slice: String::new(),
            message: payload.prompt,
            error: None,
            project,
        })),
    )?;
    let Response::Recalled(memories) = read_frame::<Response>(&mut stream)? else { return Ok(None) };
    let context = additional_context(memories);
    Ok((!context.is_empty()).then(|| {
        json!({"hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit", "additionalContext": context
        }})
        .to_string()
    }))
}

fn capture(home: &Home, envelope: &Envelope) -> anyhow::Result<Option<UnixStream>> {
    if let Ok(mut stream) = UnixStream::connect(home.socket_path()) {
        stream.set_write_timeout(Some(Duration::from_millis(5)))?;
        if write_frame(&mut stream, envelope).is_ok() {
            return Ok(Some(stream));
        }
    }
    spool::spool(home, envelope)?;
    Ok(None)
}

fn additional_context(memories: Vec<RecallMemory>) -> String {
    let mut context = String::new();
    for memory in memories {
        let prefix = format!("{}: ", memory.id);
        let available = 10_000 - context.chars().count();
        let Some(budget) = available.checked_sub(prefix.chars().count() + 1) else { break };
        if budget == 0 {
            break;
        }
        let excerpt: String = memory.content.chars().take(budget).collect();
        let _ = writeln!(context, "{prefix}{excerpt}");
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_never_truncates_a_fact_id() {
        let first: neomem_core::FactId = "fct_01M55FJ500000000000000000D".parse().unwrap();
        let second: neomem_core::FactId = "fct_01M5D6RA00000000000000000B".parse().unwrap();
        let content = "x".repeat(10_000 - first.to_string().len() - 10);
        let result = additional_context(vec![
            RecallMemory { id: first, content },
            RecallMemory { id: second, content: "second memory".to_owned() },
        ]);
        assert!(result.ends_with('\n'));
        assert_eq!(result.lines().count(), 1);
        assert!(result.chars().count() < 10_000);
    }
}
