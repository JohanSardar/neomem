use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use neomem_testkit::{FakeRules, LlmRule};
use rusqlite::Connection;
use serde_json::json;

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries, agent: &str) -> anyhow::Result<()> {
    let mut isolated = IsolatedNeomem::prepared(binaries)?;
    isolated.add_fake_rules(&FakeRules {
        llm: vec![LlmRule {
            system_contains: Some("You are neomem's memory extractor".to_owned()),
            reply: json!({"facts": [], "entities": []}).to_string(),
            ..LlmRule::default()
        }],
        ..FakeRules::default()
    })?;
    let directory = isolated.home().root().join("capture").join(agent);
    let fixture =
        crate::repo::root()?.join(format!(".agents/skills/neomem-control/fixtures/transcripts/{agent}"));
    fs::create_dir_all(&directory)?;
    let sessions = directory.join(if agent == "cc" { "projects" } else { "sessions" });
    fs::create_dir_all(&sessions)?;
    let transcript = sessions.join("rollout-capture-session.jsonl");
    let secret = ["sk-proj-", "Zq81xYvB2nT7kLm3Pq9RsUv4WxYz0AbCdEfGh"].concat();
    let text = fs::read_to_string(fixture.join("session.jsonl"))?
        .replace("capture orchard", &format!("capture orchard OPENAI_API_KEY={secret}"));
    fs::write(&transcript, text)?;
    if agent == "codex" {
        fs::write(
            sessions.join("000-unrelated.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"unrelated\"}}\n{bad}\n",
        )?;
    }
    stalled_stdin(proof, &isolated, agent)?;
    let payload = if agent == "codex" {
        serde_json::from_str(&fs::read_to_string(fixture.join("hook-without-path.json"))?)?
    } else {
        json!({"session_id":"capture-session","transcript_path":transcript})
    };
    offline_hooks(proof, &isolated, agent, &payload)?;
    isolated.start_daemon()?;
    wait_for(&isolated, "The orchard is ready")?;
    let initial = contents(&isolated)?;
    proof.claim(
        "recorded user, assistant and action reach intake",
        initial.contains("Remember the capture orchard")
            && initial.contains("The orchard is ready")
            && initial.contains("cargo test"),
        &initial,
    );
    proof.claim(
        "raw tool output and inactive pi branches never reach intake",
        !initial.contains("RAW_OUTPUT_SENTINEL") && !initial.contains("ABANDONED_BRANCH_SENTINEL"),
        &initial,
    );
    if agent == "codex" {
        proof.claim(
            "hosted web searches are captured without raw results",
            initial.contains("web_search") && !initial.contains("RAW_OUTPUT_SENTINEL"),
            &initial,
        );
    }
    proof.claim("secrets are removed before intake", !initial.contains(&secret), "key redacted");
    proof.claim(
        "spooled signal drains after daemon startup",
        neomem_proto::spool::spooled(isolated.home()).is_empty(),
        format!("{} files pending", neomem_proto::spool::spooled(isolated.home()).len()),
    );
    let db = Connection::open(isolated.home().state_db_path())?;
    let delayed: i64 =
        db.query_row("SELECT count(*) FROM jobs WHERE not_before = created_at + 300000", [], |row| {
            row.get(0)
        })?;
    let count = count_intake(&isolated)?;
    proof.claim(
        "session extraction waits five minutes after the last message",
        delayed == count,
        format!("{delayed}/{count} jobs delayed"),
    );
    let source = if agent == "cc" { "claude-code" } else { agent };
    let metadata: i64 = db.query_row("SELECT count(*) FROM intake WHERE source = ?1 AND session = 'capture-session' AND json_extract(tags, '$[0]') = 'agent-session'", [source], |row| row.get(0))?;
    proof.claim(
        "turns retain source, session and agent-session tag",
        metadata == count,
        format!("{metadata}/{count} records"),
    );
    let expected = if agent == "codex" { 5 } else { 3 };
    proof.claim("every recorded turn is captured", count == expected, format!("{count}/{expected}"));
    if agent == "codex" {
        proof.claim(
            "the environment context Codex injects as a user message is not logged as a turn",
            !contents(&isolated)?.contains("<environment_context>"),
            "",
        );
    }
    storm_and_restart(proof, &mut isolated, agent, &payload, &transcript, count)?;
    partial_and_sweep(proof, &mut isolated, agent, &payload, &fixture, &transcript, count)
}

fn offline_hooks(
    proof: &mut ProofRun,
    isolated: &IsolatedNeomem,
    agent: &str,
    payload: &serde_json::Value,
) -> anyhow::Result<()> {
    let mut large = payload.clone();
    large["tool_response"] = json!("ignored raw output ".repeat(6000));
    let started = Instant::now();
    let accepted = hook(isolated, agent, "stop", &large)?;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    proof.claim(
        "large hook payload is spooled silently within 50 ms",
        accepted && elapsed < 50.0 && !neomem_proto::spool::spooled(isolated.home()).is_empty(),
        format!("{elapsed:.3} ms"),
    );
    let started = Instant::now();
    let offline = hook(isolated, agent, "stop", payload)?;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    proof.claim(
        "offline hook exits zero silently within 50 ms",
        offline && elapsed < 50.0,
        format!("{elapsed:.3} ms"),
    );
    proof.claim(
        "offline signal is spooled",
        !neomem_proto::spool::spooled(isolated.home()).is_empty(),
        "signal pending",
    );
    Ok(())
}

fn stalled_stdin(proof: &mut ProofRun, isolated: &IsolatedNeomem, agent: &str) -> anyhow::Result<()> {
    let started = Instant::now();
    let mut stalled = Command::new(&isolated.binaries().neomem)
        .args(["hook", agent, "stop"])
        .env("NEOMEM_HOME", isolated.home().root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let held = stalled.stdin.take();
    let output = stalled.wait_with_output()?;
    drop(held);
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    proof.claim(
        "hook with stalled stdin still exits zero silently within 50 ms",
        output.status.success() && output.stdout.is_empty() && output.stderr.is_empty() && elapsed < 50.0,
        format!("{elapsed:.3} ms"),
    );
    Ok(())
}

fn storm_and_restart(
    proof: &mut ProofRun,
    isolated: &mut IsolatedNeomem,
    agent: &str,
    payload: &serde_json::Value,
    transcript: &Path,
    count: i64,
) -> anyhow::Result<()> {
    let mut latencies = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        let success = hook(isolated, agent, "tool", payload)?;
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        anyhow::ensure!(success, "online hook failed");
    }
    latencies.sort_by(f64::total_cmp);
    proof.claim(
        "online hook p95 stays below 50 ms",
        latencies[18] < 50.0,
        format!("p95 {:.3} ms", latencies[18]),
    );
    let alias = transcript
        .parent()
        .ok_or_else(|| anyhow::anyhow!("transcript has no parent"))?
        .join(".")
        .join(transcript.file_name().ok_or_else(|| anyhow::anyhow!("transcript has no name"))?);
    let signal = neomem_core::CaptureSignal {
        agent: serde_json::from_value(json!(agent))?,
        session: "capture-session".to_owned(),
        transcript_path: Some(alias),
        event: neomem_core::CaptureEvent::Stop,
    };
    isolated.call(proof, neomem_proto::Request::Capture(signal))?;
    proof.claim(
        "path aliases cannot duplicate captured turns",
        count_intake(isolated)? == count,
        format!("{count} original records"),
    );
    thread::sleep(Duration::from_millis(400));
    isolated.stop_daemon()?;
    isolated.start_daemon()?;
    thread::sleep(Duration::from_millis(400));
    proof.claim(
        "hook storm and restart capture each turn exactly once",
        count == count_intake(isolated)?,
        format!("{count} records"),
    );
    Ok(())
}

fn partial_and_sweep(
    proof: &mut ProofRun,
    isolated: &mut IsolatedNeomem,
    agent: &str,
    payload: &serde_json::Value,
    fixture: &Path,
    transcript: &Path,
    count: i64,
) -> anyhow::Result<()> {
    let tail = fs::read(fixture.join("tail.json"))?;
    let split = tail.len() / 2;
    let mut file = fs::OpenOptions::new().append(true).open(transcript)?;
    file.write_all(&tail[..split])?;
    hook(isolated, agent, "compact", payload)?;
    thread::sleep(Duration::from_millis(400));
    proof.claim(
        "crash mid-line leaves the incomplete turn unread",
        count == count_intake(isolated)?,
        "partial line retained",
    );
    isolated.stop_daemon()?;
    file.write_all(&tail[split..])?;
    let config = fs::read_to_string(isolated.home().config_path())?;
    fs::write(isolated.home().config_path(), format!("{config}\n[worker]\nsession_silence = \"10s\"\n"))?;
    isolated.start_daemon()?;
    wait_for(isolated, "Completed after crash")?;
    proof.claim(
        "sweep captures a completed partial line without a hook",
        count_intake(isolated)? == count + 1,
        contents(isolated)?,
    );
    let db = Connection::open(isolated.home().state_db_path())?;
    let configured: i64 =
        db.query_row("SELECT count(*) FROM jobs WHERE not_before = created_at + 10000", [], |row| {
            row.get(0)
        })?;
    proof.claim(
        "captured turns follow the worker session-silence configuration",
        configured == 1,
        format!("{configured} configured jobs"),
    );
    let missing_path = json!({"session_id":"capture-session"});
    hook(isolated, agent, "end", &missing_path)?;
    thread::sleep(Duration::from_millis(300));
    let db = Connection::open(isolated.home().state_db_path())?;
    let now = i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis())?;
    let pending: i64 = db.query_row(
        "SELECT count(*) FROM jobs WHERE state = 'pending' AND batch_key LIKE '%/capture-session' AND not_before > ?1",
        [now + 1000],
        |row| row.get(0),
    )?;
    proof.claim(
        "end without transcript path locates session and expedites extraction",
        pending == 0,
        format!("{pending} delayed jobs"),
    );
    let unsignaled = fs::read_to_string(fixture.join("session.jsonl"))?
        .replace("capture-session", "nohook-session")
        .replace("Remember the capture orchard", "Unsignaled capture orchard");
    isolated.stop_daemon()?;
    fs::write(
        transcript
            .parent()
            .ok_or_else(|| anyhow::anyhow!("transcript has no directory"))?
            .join("nohook-session.jsonl"),
        unsignaled,
    )?;
    isolated.start_daemon()?;
    wait_for(isolated, "Unsignaled capture orchard")?;
    proof.claim(
        "a session with no hook is discovered",
        count_intake(isolated)? == count + 1 + if agent == "codex" { 5 } else { 3 },
        contents(isolated)?,
    );
    let positions: i64 =
        db.query_row("SELECT count(*) FROM transcript_positions WHERE position > 0", [], |row| row.get(0))?;
    proof.claim("byte positions persist in state.db", positions == 2, format!("{positions} transcripts"));
    if agent == "pi" {
        reactivate_pi_branch(proof, isolated, fixture, transcript)?;
    }
    proof.attach_logs(&isolated.logs());
    Ok(())
}

fn reactivate_pi_branch(
    proof: &mut ProofRun,
    isolated: &mut IsolatedNeomem,
    fixture: &Path,
    transcript: &Path,
) -> anyhow::Result<()> {
    let before = count_intake(isolated)?;
    fs::OpenOptions::new()
        .append(true)
        .open(transcript)?
        .write_all(&fs::read(fixture.join("reactivate.jsonl"))?)?;
    isolated.call(
        proof,
        neomem_proto::Request::Capture(neomem_core::CaptureSignal {
            agent: neomem_core::CaptureAgent::Pi,
            session: "capture-session".to_owned(),
            transcript_path: Some(transcript.to_path_buf()),
            event: neomem_core::CaptureEvent::Stop,
        }),
    )?;
    wait_for(isolated, "Selected the alternate branch")?;
    proof.claim(
        "reactivating an older pi branch captures its previously skipped ancestor once",
        count_intake(isolated)? == before + 2 && contents(isolated)?.contains("ABANDONED_BRANCH_SENTINEL"),
        contents(isolated)?,
    );
    isolated.stop_daemon()?;
    isolated.start_daemon()?;
    thread::sleep(Duration::from_millis(300));
    proof.claim(
        "pi ancestry checkpoints survive restart without duplicates",
        count_intake(isolated)? == before + 2,
        contents(isolated)?,
    );
    Ok(())
}

fn hook(
    isolated: &IsolatedNeomem,
    agent: &str,
    event: &str,
    payload: &serde_json::Value,
) -> anyhow::Result<bool> {
    let mut child = Command::new(&isolated.binaries().neomem)
        .args(["hook", agent, event])
        .env("NEOMEM_HOME", isolated.home().root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let written =
        child.stdin.take().is_some_and(|mut input| serde_json::to_writer(&mut input, payload).is_ok());
    let output = child.wait_with_output()?;
    Ok(written && output.status.success() && output.stdout.is_empty() && output.stderr.is_empty())
}

fn contents(isolated: &IsolatedNeomem) -> anyhow::Result<String> {
    let db = Connection::open(isolated.home().state_db_path())?;
    Ok(db.query_row("SELECT coalesce(group_concat(content, char(10)), '') FROM intake", [], |row| {
        row.get(0)
    })?)
}

fn count_intake(isolated: &IsolatedNeomem) -> anyhow::Result<i64> {
    let db = Connection::open(isolated.home().state_db_path())?;
    Ok(db.query_row("SELECT count(*) FROM intake", [], |row| row.get(0))?)
}

fn wait_for(isolated: &IsolatedNeomem, text: &str) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if contents(isolated)?.contains(text) && neomem_proto::spool::spooled(isolated.home()).is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    anyhow::bail!("capture did not log {text} and drain the spool")
}
