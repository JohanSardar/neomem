use super::recall_support as support;
use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = support::setup(proof, binaries)?;
    for agent in ["cc", "codex"] {
        let (off, elapsed) = hook(&neomem, agent, "off", "/tmp")?;
        proof.claim(
            &format!("{agent} recall off stays silent within 50 ms"),
            off.status.success() && off.stdout.is_empty() && elapsed.as_millis() < 50,
            format!("{:.3} ms", elapsed.as_secs_f64() * 1000.0),
        );
        neomem.neomem(&["recall", "on", "--agent", agent])?;
        let (on, elapsed) = hook(&neomem, agent, "on", "/tmp")?;
        let output: Value = serde_json::from_slice(&on.stdout).unwrap_or(Value::Null);
        let context = output["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default();
        proof.claim(
            &format!("{agent} prompt returns approved memories in hook JSON"),
            on.status.success()
                && output["hookSpecificOutput"]["hookEventName"] == "UserPromptSubmit"
                && context.contains(support::SIGNING)
                && context.chars().count() <= 10_000,
            format!("{} characters; {:.3} ms", context.chars().count(), elapsed.as_secs_f64() * 1000.0),
        );
        neomem.neomem(&["recall", "off", "--agent", agent])?;
    }
    let calls = neomem.fake_model_calls()?;
    let slices: Vec<_> =
        calls.iter().filter(|call| call.route == "/v1/systemone").map(|call| call.body.to_string()).collect();
    proof.claim(
        "recall uses the captured transcript slice without raw tool output",
        slices.iter().any(|slice| slice.contains("capture orchard"))
            && slices.iter().all(|slice| !slice.contains("RAW_OUTPUT_SENTINEL")),
        "sanitized session turns in judge state",
    );
    neomem.neomem(&["recall", "on", "--project", "/tmp"])?;
    let (other, _) = hook(&neomem, "cc", "other", "/")?;
    proof.claim("project switch does not enable another project", other.stdout.is_empty(), "other cwd");
    let (project, _) = hook(&neomem, "cc", "project", "/tmp")?;
    proof.claim("project switch enables its own prompt", !project.stdout.is_empty(), "matching cwd");
    let (writer, reader) = std::os::unix::net::UnixStream::pair()?;
    drop(reader);
    let (closed, elapsed) =
        hook_output(&neomem, "cc", "closed-output", "/tmp", Stdio::from(std::os::fd::OwnedFd::from(writer)))?;
    let injected: i64 = support::db(&neomem)?.query_row(
        "SELECT count(*) FROM recall_injected WHERE agent = 'cc' AND session = 'cc-closed-output'",
        [],
        |row| row.get(0),
    )?;
    proof.claim(
        "a closed context pipe still exits zero silently",
        injected > 0
            && closed.status.success()
            && closed.stderr.is_empty()
            && elapsed < Duration::from_millis(3600),
        closed.status,
    );
    oversized_context(proof, &neomem)?;
    neomem.add_fake_rules(&neomem_testkit::FakeRules {
        llm: vec![support::planner(10_000, 0)],
        ..neomem_testkit::FakeRules::default()
    })?;
    let (slow, elapsed) = hook(&neomem, "cc", "slow", "/tmp")?;
    proof.claim(
        "slow planner fails open within the hard hook deadline",
        slow.status.success() && elapsed < Duration::from_millis(3600),
        format!("{:.3} ms", elapsed.as_secs_f64() * 1000.0),
    );
    neomem.stop_daemon()?;
    let (down, elapsed) = hook(&neomem, "cc", "down", "/tmp")?;
    proof.claim(
        "offline recall fails open and spools capture",
        down.status.success()
            && down.stdout.is_empty()
            && !neomem_proto::spool::spooled(neomem.home()).is_empty()
            && elapsed < Duration::from_millis(3600),
        format!("{:.3} ms", elapsed.as_secs_f64() * 1000.0),
    );
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn hook(
    neomem: &IsolatedNeomem,
    agent: &str,
    session: &str,
    cwd: &str,
) -> anyhow::Result<(Output, Duration)> {
    hook_output(neomem, agent, session, cwd, Stdio::piped())
}

fn hook_output(
    neomem: &IsolatedNeomem,
    agent: &str,
    session: &str,
    cwd: &str,
    stdout: Stdio,
) -> anyhow::Result<(Output, Duration)> {
    let fixture = crate::repo::root()?
        .join(format!(".agents/skills/neomem-control/fixtures/transcripts/{agent}/session.jsonl"));
    let transcript = neomem.home().root().join(format!("{agent}-{session}.jsonl"));
    std::fs::write(
        &transcript,
        std::fs::read_to_string(fixture)?.replace("capture-session", &format!("{agent}-{session}")),
    )?;
    let started = Instant::now();
    let mut child = Command::new(&neomem.binaries().neomem)
        .args(["hook", agent, "prompt"])
        .env("NEOMEM_HOME", neomem.home().root())
        .stdin(Stdio::piped())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(json!({"session_id":format!("{agent}-{session}"), "cwd":cwd, "prompt":support::PROMPT, "transcript_path":transcript}).to_string().as_bytes())?;
    }
    Ok((child.wait_with_output()?, started.elapsed()))
}

fn oversized_context(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let facts = neomem.home().root().join("facts/2026/10");
    let mut originals = Vec::new();
    for entry in std::fs::read_dir(facts)? {
        let path = entry?.path();
        let original = std::fs::read_to_string(&path)?;
        let body = original.rsplit("---").next().unwrap_or_default();
        std::fs::write(&path, format!("{original}{}", format!("{body} 🧠").repeat(200)))?;
        originals.push((path, original));
    }
    neomem.neomem(&["rebuild"])?;
    let (output, _) = hook(neomem, "cc", "oversized", "/tmp")?;
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let context = value["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default();
    proof.claim(
        "oversized Unicode memories are capped at 10000 characters",
        output.status.success() && context.chars().count() == 10_000 && context.starts_with("fct_"),
        context.chars().count(),
    );
    for (path, original) in originals {
        std::fs::write(path, original)?;
    }
    neomem.neomem(&["rebuild"])?;
    Ok(())
}
