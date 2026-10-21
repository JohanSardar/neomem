use std::fs;
use std::process::Output;
use std::thread;
use std::time::{Duration, Instant};

use neomem_core::Source;

use neomem_proto::{InstallOutcome, InstallReport, RawQuery, Request, Response};
use neomem_testkit::{FakeRules, LlmRule, RecordedCall, RuleToolCall};
use serde_json::Value;

use crate::agents::{Agent, AgentSandbox};
use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::{recall_support, support};
use crate::{repo, seed};

const SHIP_DATE: &str = "fct_01M5D6RA00000000000000000B";
const PROMPT: &str = "What is the Edge ship date? Check my notes.";
const ANSWER: &str = "Edge ships on November 1, 2026.";
const SKILL_FROM_ROOT: &str = "integrations/skills/neomem/SKILL.md";
const CAPTURE_WAIT: Duration = Duration::from_secs(10);
const SKILL_DESCRIPTION: &str = "Long-term memory shared by every agent session on this machine";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = seed::start_with_vault_fixture("workspace", binaries)?;
    support::wait_for_index(&neomem, proof, 13, 7)?;
    let sandbox = AgentSandbox::new(&neomem)?;
    let first_daemon = daemon_pid(&neomem, proof)?;
    let first = install(&sandbox)?;
    proof.claim(
        "neomem install wires the integrations, the skill, Claude Code, Codex, pi and the service",
        first.as_ref().is_some_and(|report| {
            every_part_done(report, &["integrations", "Claude Code", "Codex", "pi", "service"])
        }),
        format!("{first:?}"),
    );
    let second = install(&sandbox)?;
    proof.claim(
        "running install again succeeds and installs nothing twice",
        second
            .as_ref()
            .is_some_and(|report| report.steps.iter().all(|step| step.outcome == InstallOutcome::Done))
            && installed_once(&sandbox)?,
        format!("{second:?}"),
    );
    the_skill_is_where_each_agent_reads_skills(proof, &sandbox)?;
    the_service_manager_runs_the_daemon(proof, &neomem, &sandbox, first_daemon)?;
    proof.claim(
        "Codex trusts neomem's hooks, so it runs them",
        fs::read_to_string(sandbox.codex_dir().join("config.toml"))?.matches("trusted_hash").count() >= 3,
        "",
    );
    neomem.neomem(&["recall", "on"])?;
    let mut replies = agent_replies();
    replies.push(recall_support::planner(0, 0));
    neomem.add_fake_rules(&FakeRules {
        judge: vec![
            recall_support::rule("long-term memory", None, 0.9),
            recall_support::rule("this memory", None, 0.9),
        ],
        llm: replies,
        ..FakeRules::default()
    })?;
    for agent in Agent::ALL {
        a_session_captures_and_recalls(proof, &neomem, &sandbox, agent)?;
    }
    pi_answers_from_the_neomem_tool(proof, &neomem, &sandbox)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn install(sandbox: &AgentSandbox) -> anyhow::Result<Option<InstallReport>> {
    let output = sandbox.run("neomem", &["--json", "install"])?;
    Ok(match serde_json::from_slice::<Response>(&output.stdout) {
        Ok(Response::Installed(report)) => Some(report),
        _ => None,
    })
}

fn every_part_done(report: &InstallReport, parts: &[&str]) -> bool {
    report.steps.iter().all(|step| step.outcome == InstallOutcome::Done)
        && parts.iter().all(|part| report.steps.iter().any(|step| step.part == *part))
}

fn installed_once(sandbox: &AgentSandbox) -> anyhow::Result<bool> {
    let claude = String::from_utf8_lossy(&sandbox.run("claude", &["plugin", "list"])?.stdout).into_owned();
    let codex_config = fs::read_to_string(sandbox.codex_dir().join("config.toml"))?;
    Ok(claude.matches("neomem@neomem").count() == 1
        && codex_config.matches("[marketplaces.neomem]").count() == 1
        && codex_config.matches("[plugins.\"neomem@neomem\"]").count() == 1)
}

fn the_skill_is_where_each_agent_reads_skills(
    proof: &mut ProofRun,
    sandbox: &AgentSandbox,
) -> anyhow::Result<()> {
    let skill = fs::read_to_string(repo::root()?.join(SKILL_FROM_ROOT))?;
    let linked = [
        sandbox.home().join(".agents/skills/neomem/SKILL.md"),
        sandbox.claude_dir().join("skills/neomem/SKILL.md"),
    ];
    proof.claim(
        "the skill is linked into ~/.agents/skills (Codex, pi) and Claude Code's skills folder",
        linked.iter().all(|path| fs::read_to_string(path).is_ok_and(|text| text == skill)),
        "",
    );
    Ok(())
}

fn daemon_pid(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<Option<u32>> {
    Ok(match neomem.call(proof, Request::Status)? {
        Response::Status(report) => Some(report.pid),
        _ => None,
    })
}

fn the_service_manager_runs_the_daemon(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    sandbox: &AgentSandbox,
    first_daemon: Option<u32>,
) -> anyhow::Result<()> {
    let active = sandbox.has_service_manager()
        && sandbox.run("systemctl", &["--user", "is-active", "neomemd.service"])?.status.success();
    let serving = daemon_pid(neomem, proof)?;
    proof.claim(
        "the systemd user unit runs neomemd for this home in place of the daemon that was running",
        active && serving.is_some() && serving != first_daemon,
        format!("before {first_daemon:?}, after {serving:?}"),
    );
    Ok(())
}

fn agent_replies() -> Vec<LlmRule> {
    Agent::ALL
        .iter()
        .map(|agent| LlmRule {
            system_contains: Some(agent.system_marker().to_owned()),
            reply: ANSWER.to_owned(),
            ..LlmRule::default()
        })
        .collect()
}

fn calls_of(neomem: &IsolatedNeomem, agent: Agent) -> anyhow::Result<Vec<RecordedCall>> {
    Ok(neomem
        .fake_model_calls()?
        .into_iter()
        .filter(|call| call.body.to_string().contains(agent.system_marker()))
        .collect())
}

fn mentions_neomem_tool(body: &Value) -> bool {
    let offered = body["tools"].as_array().is_some_and(|tools| {
        tools.iter().any(|tool| {
            let name =
                tool["name"].as_str().or_else(|| tool["function"]["name"].as_str()).unwrap_or_default();
            name.starts_with("mcp__neomem") || name == "neomem_search"
        })
    });
    offered || body.to_string().contains("mcp__plugin_neomem_neomem__search")
}

fn captured(neomem: &IsolatedNeomem, proof: &mut ProofRun, agent: Agent) -> anyhow::Result<bool> {
    let deadline = Instant::now() + CAPTURE_WAIT;
    loop {
        let query = RawQuery {
            text: String::new(),
            from: None,
            to: None,
            source: Some(Source::new(agent.source())?),
            limit: 50,
        };
        let texts: Vec<String> = match neomem.call(proof, Request::Raw(query))? {
            Response::RawHits(hits) => hits.into_iter().map(|hit| hit.content).collect(),
            _ => Vec::new(),
        };
        if texts.iter().any(|text| text.contains(PROMPT)) && texts.iter().any(|text| text.contains(ANSWER)) {
            return Ok(true);
        }
        if Instant::now() > deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn a_session_captures_and_recalls(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    sandbox: &AgentSandbox,
    agent: Agent,
) -> anyhow::Result<()> {
    let output: Output = sandbox.session(agent, PROMPT)?;
    let name = agent.program();
    let answered = output.status.success() && String::from_utf8_lossy(&output.stdout).contains(ANSWER);
    let calls = calls_of(neomem, agent)?;
    proof.claim(
        &format!("a scripted {name} session runs against the fake model and answers"),
        answered && !calls.is_empty(),
        format!(
            "{} model calls; {}",
            calls.len(),
            String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default()
        ),
    );
    proof.claim(
        &format!("{name} sees the neomem skill"),
        calls.iter().any(|call| call.body.to_string().contains(SKILL_DESCRIPTION)),
        "",
    );
    proof.claim(
        &format!("{name} offers the neomem search tool to its model (MCP for Claude Code and Codex, the extension for pi)"),
        calls.iter().any(|call| mentions_neomem_tool(&call.body)),
        "",
    );
    let was_captured = captured(neomem, proof, agent)?;
    proof.claim(
        &format!("{name}'s prompt and answer reach the intake log through capture"),
        was_captured,
        "",
    );
    proof.claim(
        &format!("{name}'s model call carries the recalled ship-date fact with its fct_ ID"),
        calls.iter().any(|call| call.body.to_string().contains(SHIP_DATE)),
        "",
    );
    Ok(())
}

const TOOL_PROMPT: &str = "Look the Edge ship date up in memory with the neomem tool.";

fn tool_results_of(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "tool")
        .map(|message| message["content"].to_string())
        .collect()
}

fn pi_answers_from_the_neomem_tool(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    sandbox: &AgentSandbox,
) -> anyhow::Result<()> {
    neomem.neomem(&["recall", "off"])?;
    let scripted = |step: usize, tool_calls: Vec<RuleToolCall>, reply: &str| LlmRule {
        system_contains: Some(Agent::Pi.system_marker().to_owned()),
        user_contains: Some(TOOL_PROMPT.to_owned()),
        step: Some(step),
        tool_calls,
        reply: reply.to_owned(),
        ..LlmRule::default()
    };
    let search = RuleToolCall {
        name: "neomem_search".to_owned(),
        arguments: serde_json::json!({ "text": "Edge ship date" }),
    };
    neomem.add_fake_rules(&FakeRules {
        llm: vec![scripted(0, vec![search], ""), scripted(1, Vec::new(), ANSWER)],
        ..FakeRules::default()
    })?;
    let output = sandbox.session(Agent::Pi, TOOL_PROMPT)?;
    let results: Vec<String> = calls_of(neomem, Agent::Pi)?
        .iter()
        .filter(|call| call.body.to_string().contains(TOOL_PROMPT))
        .flat_map(|call| tool_results_of(&call.body))
        .collect();
    proof.claim(
        "pi calls neomem_search through the extension and the answer, with the ship-date fct_ ID, reaches its model",
        output.status.success() && results.iter().any(|result| result.contains(SHIP_DATE)),
        format!("{} tool results", results.len()),
    );
    Ok(())
}
