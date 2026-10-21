use super::recall_support;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use serde_json::json;
use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn prove(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    neomem.neomem(&["recall", "on", "--agent", "pi"])?;
    let installation = Command::new("npm").args(["prefix", "-g"]).output()?;
    let installation = String::from_utf8(installation.stdout)?;
    let cli = std::path::Path::new(installation.trim()).join("bin/pi");
    let directory = neomem.home().root().join("pi-proof");
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join("models.json"),
        json!({"providers":{"neomem-proof":{
            "baseUrl":format!("http://{}/v1", neomem.fake_models_address()?),
            "api":"openai-completions", "apiKey":"isolated-fake",
            "models":[{"id":"pi-hook-proof", "reasoning":false, "contextWindow":32768, "maxTokens":256}]
        }}})
        .to_string(),
    )?;
    let before = neomem.fake_model_calls()?.len();
    let package = crate::repo::root()?.join("integrations/pi");
    let install = Command::new("pnpm")
        .args(["install", "--frozen-lockfile", "--ignore-scripts"])
        .current_dir(&package)
        .output()?;
    anyhow::ensure!(
        install.status.success(),
        "installing pi extension dependencies: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    transport(proof, neomem, &package)?;
    let extension = package.join("index.js");
    let mut child = Command::new("node")
        .arg(cli)
        .args([
            "--print",
            "--mode",
            "json",
            "--provider",
            "neomem-proof",
            "--model",
            "pi-hook-proof",
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
        ])
        .arg("--extension")
        .arg(extension)
        .arg("--session-dir")
        .arg(directory.join("sessions"))
        .arg(recall_support::PROMPT)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", &directory)
        .env("PI_CODING_AGENT_DIR", &directory)
        .env("NEOMEM_HOME", neomem.home().root())
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    while child.try_wait()?.is_none() && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let output = child.wait_with_output()?;
    let calls = neomem.fake_model_calls()?;
    let injected = calls[before..].iter().any(|call| {
        call.route == "/v1/chat/completions"
            && call.body["model"] == "pi-hook-proof"
            && call.body.to_string().contains(recall_support::SIGNING)
    });
    proof.claim(
        "real headless pi receives recalled IDs in model context",
        output.status.success() && injected,
        format!("status {}; injected {injected}; {}", output.status, String::from_utf8_lossy(&output.stderr)),
    );
    let captured = captured(neomem)?;
    proof.claim("real pi turn_end and agent_end capture its transcript", captured > 0, captured);
    Ok(())
}

fn transport(proof: &mut ProofRun, neomem: &IsolatedNeomem, package: &std::path::Path) -> anyhow::Result<()> {
    let output = Command::new("node")
        .arg(package.join("proof.mjs"))
        .arg(neomem.home().root().join("pi-wire-proof"))
        .output()?;
    proof.claim(
        "pi shares one socket, sends both signals synchronously and bounds recall",
        output.status.success(),
        format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)),
    );
    Ok(())
}

fn captured(neomem: &IsolatedNeomem) -> anyhow::Result<i64> {
    let connection = recall_support::db(neomem)?;
    let started = Instant::now();
    loop {
        let count = connection.query_row(
            "SELECT count(*) FROM intake WHERE source = 'pi' AND content LIKE 'assistant:%'",
            [],
            |row| row.get(0),
        )?;
        if count > 0 || started.elapsed() > Duration::from_secs(2) {
            return Ok(count);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
