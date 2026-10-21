use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::Context;
use neomem_proto::{Request, SearchQuery};
use serde_json::{Value, json};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::{seed, timing};

pub(crate) struct StdioClient {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    serial: u64,
}

impl StdioClient {
    pub(crate) fn open(neomem: &IsolatedNeomem) -> anyhow::Result<Self> {
        let mut child = Command::new(&neomem.binaries().neomem)
            .arg("mcp")
            .env("NEOMEM_HOME", neomem.home().root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().context("MCP stdin")?;
        let output = BufReader::new(child.stdout.take().context("MCP stdout")?);
        Ok(Self { child, input, output, serial: 0 })
    }

    pub(crate) fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.serial += 1;
        writeln!(self.input, "{}", message(self.serial, method, params))?;
        self.input.flush()?;
        let mut line = String::new();
        self.output.read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn message(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

fn initialize() -> Value {
    json!({"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"proof","version":"1"}})
}

fn http(neomem: &IsolatedNeomem, request: Value) -> anyhow::Result<Value> {
    Ok(ureq::post(&format!("http://{}/mcp", neomem.http_address()?))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-03-26")
        .send_json(request)?.body_mut().read_json()?)
}

fn body(response: &Value) -> anyhow::Result<Value> {
    let text = response["result"]["content"][0]["text"].as_str().context("tool content")?;
    Ok(serde_json::from_str(text)?)
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = seed::start_with_vault_fixture("workspace", binaries)?;
    support::wait_for_index(&neomem, proof, 13, 7)?;
    let mut stdio = StdioClient::open(&neomem)?;
    let initialized = stdio.call("initialize", initialize())?;
    let http_initialized = http(&neomem, message(1, "initialize", initialize()))?;
    proof.claim("both MCP transports initialize", initialized["result"]["capabilities"].is_object()
        && http_initialized["result"]["capabilities"].is_object(), "");
    let listed = stdio.call("tools/list", json!({}))?;
    let remote = http(&neomem, message(2, "tools/list", json!({})))?;
    let tools = listed["result"]["tools"].as_array().context("tools array")?;
    let required = ["search", "add", "raw", "entity", "show", "history", "resolve", "useful"];
    proof.claim("stdio and HTTP expose the same tools and object input schemas",
        listed["result"] == remote["result"] && required.iter().all(|name| tools.iter().any(|tool| tool["name"] == *name))
        && tools.iter().all(|tool| tool["inputSchema"]["type"] == "object" && tool["inputSchema"]["properties"].is_object()), listed.to_string());
    let query = "how was the Edge code signing error fixed";
    let direct = neomem.call(proof, Request::Search(SearchQuery { text: query.into(), ..SearchQuery::default() }))?;
    let found = body(&stdio.call("tools/call", json!({"name":"search","arguments":{"text":query}}))?)?;
    let direct = serde_json::to_value(direct)?;
    let ids = |value: &Value| value["body"]["hits"].as_array().map(|hits| hits.iter().map(|hit| hit["fact"]["id"].clone()).collect::<Vec<_>>());
    proof.claim("MCP search returns the socket search fact IDs", ids(&found).is_some() && ids(&found) == ids(&direct), found.to_string());
    for (name, args, reply) in [
        ("show", json!({"id":"fct_01M55FJ500000000000000000D"}), "shown"),
        ("history", json!({"id":"fct_01M55FJ500000000000000000D"}), "history"),
        ("entity", json!({"name":"Edge"}), "entity"),
        ("resolve", json!({"name":"Edge"}), "resolved"),
        ("useful", json!({"id":"fct_01M55FJ500000000000000000D"}), "marked_useful"),
        ("add", json!({"content":"MCP intake proof marker","source":"mcp"}), "added"),
        ("raw", json!({"text":"MCP intake proof marker"}), "raw_hits"),
    ] {
        let response = body(&stdio.call("tools/call", json!({"name":name,"arguments":args}))?)?;
        proof.claim(&format!("MCP {name} answers through the dispatcher"), response["reply"] == reply
            && (name != "raw" || response["body"].as_array().is_some_and(|hits| !hits.is_empty())), response.to_string());
    }
    let bad = stdio.call("tools/call", json!({"name":"show","arguments":{"id":42}}))?;
    proof.claim("bad tool arguments give an error", bad["error"].is_object() || bad["result"]["isError"] == true, bad.to_string());
    let latency = timing::measure(20, || { stdio.call("tools/call", json!({"name":"search","arguments":{"text":query}}))?; Ok(()) })?;
    proof.claim("MCP search stdio round trip measured with fake models", true, latency);
    neomem.stop_daemon()?;
    for _ in 0..2 {
        let stopped = stdio.call("tools/call", json!({"name":"search","arguments":{"text":query}}))?;
        proof.claim("stopped daemon returns a tool error and relay stays alive", stopped["result"]["isError"] == true, stopped.to_string());
    }
    proof.attach_logs(&neomem.logs());
    Ok(())
}
