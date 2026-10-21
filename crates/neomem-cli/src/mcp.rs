use std::io::{BufRead, Write};
use std::time::Duration;

use neomem_proto::Home;
use serde_json::{Value, json};

pub fn run() -> anyhow::Result<()> {
    let home = Home::from_env().ok_or_else(|| anyhow::anyhow!("NEOMEM_HOME or HOME is required"))?;
    let agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().new_agent();
    let mut protocol = "2025-03-26".to_owned();
    let input = std::io::stdin();
    let mut output = std::io::stdout().lock();
    for line in input.lock().lines() {
        let line = line?;
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                writeln!(
                    output,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}})
                )?;
                output.flush()?;
                continue;
            }
        };
        let response = forward(&agent, &home, &message, &protocol);
        if let Some(id) = message.get("id") {
            let response = response.unwrap_or_else(|error| {
                if message["method"] == "tools/call" {
                    json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":format!("daemon unavailable: {error}")}]}})
                } else {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":format!("daemon unavailable: {error}")}})
                }
            });
            if message["method"] == "initialize"
                && let Some(version) = response["result"]["protocolVersion"].as_str()
            {
                version.clone_into(&mut protocol);
            }
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn forward(agent: &ureq::Agent, home: &Home, message: &Value, protocol: &str) -> anyhow::Result<Value> {
    let protocol = if message["method"] == "initialize" {
        message["params"]["protocolVersion"].as_str().unwrap_or(protocol)
    } else {
        protocol
    };
    let address = std::fs::read_to_string(home.http_address_path())?;
    let address: std::net::SocketAddr = address.trim().parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "MCP daemon address must be loopback");
    let mut response = agent
        .post(&format!("http://{address}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", protocol)
        .send_json(message)?;
    if message.get("id").is_none() {
        return Ok(Value::Null);
    }
    Ok(response.body_mut().read_json()?)
}
