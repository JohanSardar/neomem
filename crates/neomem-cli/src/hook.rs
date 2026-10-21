use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::{CaptureAgent, CaptureEvent, CaptureSignal};
use neomem_proto::{Envelope, Home, Request, spool, write_frame};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct HookPayload {
    #[serde(default, alias = "sessionId", alias = "thread_id")]
    session_id: String,
    #[serde(default, alias = "session_file")]
    transcript_path: Option<std::path::PathBuf>,
}

pub fn run(arguments: &[String]) {
    let started = Instant::now();
    let arguments = arguments.to_vec();
    let (sender, receiver) = mpsc::channel();
    let _ = thread::Builder::new().spawn(move || {
        let mut body = String::new();
        if std::io::stdin().take(4_194_305).read_to_string(&mut body).is_ok() {
            let _ = send(&arguments, &body, started);
        }
        let _ = sender.send(());
    });
    let _ = receiver.recv_timeout(Duration::from_millis(20));
}

fn send(arguments: &[String], body: &str, started: Instant) -> anyhow::Result<()> {
    let agent: CaptureAgent =
        serde_json::from_value(Value::String(arguments.first().cloned().unwrap_or_default()))?;
    let event: CaptureEvent =
        serde_json::from_value(Value::String(arguments.get(1).cloned().unwrap_or_default()))?;
    let payload: HookPayload = serde_json::from_str(body)?;
    let signal =
        CaptureSignal { agent, session: payload.session_id, transcript_path: payload.transcript_path, event };
    let home = Home::from_env().ok_or_else(|| anyhow::anyhow!("missing home"))?;
    let envelope = Envelope::current(Request::Capture(signal));
    if started.elapsed() >= Duration::from_millis(30) {
        return Ok(());
    }
    if let Ok(mut stream) = UnixStream::connect(home.socket_path()) {
        stream.set_write_timeout(Some(Duration::from_millis(5)))?;
        if write_frame(&mut stream, &envelope).is_ok() {
            return Ok(());
        }
    }
    spool::spool(&home, &envelope)?;
    Ok(())
}
