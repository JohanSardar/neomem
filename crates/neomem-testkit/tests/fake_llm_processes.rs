#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

use neomem_testkit::FakeModelServer;
use serde_json::{Value, json};

fn spawn(mode: &str) -> Child {
    let brain = format!("http://{}", FakeModelServer::bind(0).unwrap().serve_in_background().unwrap());
    Command::new(env!("CARGO_BIN_EXE_neomem-fake-llm"))
        .args(["--brain", &brain, mode])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
}

#[test]
fn the_fake_command_answers_the_request_on_stdin() {
    let mut child = spawn("command");
    let request = json!({ "system": "Answer with OK.", "messages": [{ "role": "user", "content": "ping" }] });
    child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "OK");
}

#[test]
fn the_fake_codex_app_server_needs_initialize_then_streams_a_turn() {
    let mut child = spawn("codex-app-server");
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut send = |message: Value| writeln!(input, "{message}").unwrap();
    send(json!({ "id": 1, "method": "thread/start", "params": {} }));
    assert_eq!(next(&mut output)["error"]["message"], "Not initialized");
    send(
        json!({ "id": 2, "method": "initialize", "params": { "clientInfo": { "name": "test", "version": "0" } } }),
    );
    assert!(next(&mut output)["result"]["userAgent"].is_string());
    send(
        json!({ "id": 3, "method": "thread/start", "params": { "developerInstructions": "Answer with OK." } }),
    );
    let thread = next(&mut output)["result"]["thread"]["id"].as_str().unwrap().to_owned();
    send(json!({ "id": 4, "method": "turn/start",
        "params": { "threadId": thread, "input": [{ "type": "text", "text": "ping", "text_elements": [] }] } }));
    let mut streamed = String::new();
    loop {
        let message = next(&mut output);
        match message["method"].as_str() {
            Some("item/agentMessage/delta") => {
                streamed.push_str(message["params"]["delta"].as_str().unwrap());
            }
            Some("turn/completed") => break assert_eq!(message["params"]["turn"]["status"], "completed"),
            _ => {}
        }
    }
    assert_eq!(streamed, "OK");
    drop(input);
    assert!(child.wait().unwrap().success(), "the fake exits when its input closes");
}

fn next(output: &mut BufReader<ChildStdout>) -> Value {
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
