use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value, json};

const ANSWER_WAIT: Duration = Duration::from_secs(20);

struct AppServer {
    child: Child,
    input: ChildStdin,
    answers: Receiver<Value>,
    next_id: u64,
}

impl AppServer {
    fn start(cwd: &Path) -> Result<Self, String> {
        let mut child = Command::new("codex")
            .arg("app-server")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("starting codex app-server: {error}"))?;
        let input = child.stdin.take().ok_or("codex app-server has no stdin")?;
        let output = child.stdout.take().ok_or("codex app-server has no stdout")?;
        let (sender, answers) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str::<Value>(&line)
                    && sender.send(message).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self { child, input, answers, next_id: 0 })
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        writeln!(self.input, "{message}").map_err(|error| format!("writing to codex app-server: {error}"))
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let message = self
                .answers
                .recv_timeout(ANSWER_WAIT)
                .map_err(|_| format!("codex app-server did not answer {method}"))?;
            if message["id"] != id {
                continue;
            }
            if let Some(problem) = message.get("error") {
                return Err(format!("{method}: {problem}"));
            }
            return Ok(message["result"].clone());
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn trust(plugin: &str, cwd: &Path) -> Result<usize, String> {
    let mut server = AppServer::start(cwd)?;
    server.request(
        "initialize",
        &json!({ "clientInfo": { "name": "neomem", "version": env!("CARGO_PKG_VERSION") } }),
    )?;
    server.send(&json!({ "jsonrpc": "2.0", "method": "initialized" }))?;
    let listed = server.request("hooks/list", &json!({ "cwds": [cwd] }))?;
    let trusted: Map<String, Value> = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| entry["hooks"].as_array().cloned().unwrap_or_default())
        .filter(|hook| hook["pluginId"] == plugin)
        .filter_map(|hook| {
            let key = hook["key"].as_str()?.to_owned();
            Some((key, json!({ "trusted_hash": hook["currentHash"] })))
        })
        .collect();
    if trusted.is_empty() {
        return Err(format!("codex lists no hooks for {plugin}"));
    }
    let count = trusted.len();
    server.request(
        "config/batchWrite",
        &json!({
            "edits": [{ "keyPath": "hooks.state", "value": trusted, "mergeStrategy": "upsert" }],
            "reloadUserConfig": true
        }),
    )?;
    Ok(count)
}
