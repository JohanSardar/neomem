use std::collections::BTreeSet;
use std::fs;

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::{repo, seed};

const SKILL_FROM_ROOT: &str = "integrations/skills/neomem/SKILL.md";
const SHIP_DATE: &str = "fct_01M5D6RA00000000000000000B";
const AGENT_COMMANDS: &[&str] =
    &["search", "show", "history", "entity", "resolve", "raw", "tags", "useful", "add", "correct"];
const OPERATOR_COMMANDS: &[&str] = &[
    "status",
    "recall",
    "hook",
    "rebuild",
    "dream",
    "reextract",
    "models",
    "login",
    "help",
    "link",
    "mcp",
    "install",
];
const MOST_LINES: usize = 150;
const MOST_DESCRIPTION: usize = 1024;

struct Example {
    line: String,
    ran: bool,
    json: bool,
    output: String,
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let skill = fs::read_to_string(repo::root()?.join(SKILL_FROM_ROOT))?;
    the_header_names_and_describes_the_skill(proof, &skill);
    let neomem = seed::start_with_vault_fixture("workspace", binaries)?;
    support::wait_for_index(&neomem, proof, 13, 7)?;
    let examples = run_examples(&neomem, &skill)?;
    every_example_runs(proof, &examples);
    proof.claim(
        "the first search example finds the current Edge ship date",
        examples.iter().find(|example| example.line.starts_with("neomem search")).is_some_and(|example| {
            example.output.find(SHIP_DATE).is_some_and(|at| !example.output[..at].contains("fct_"))
        }),
        "",
    );
    every_agent_command_is_taught(proof, &neomem, &skill)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn the_header_names_and_describes_the_skill(proof: &mut ProofRun, skill: &str) {
    let header =
        skill.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")).map(|(header, _)| header);
    let field = |name: &str| {
        header
            .and_then(|header| header.lines().find_map(|line| line.strip_prefix(&format!("{name}: "))))
            .unwrap_or("")
    };
    let description = field("description");
    proof.claim(
        "the skill header is named neomem after its folder and describes when to use it in at most 1024 characters",
        field("name") == "neomem" && !description.is_empty() && description.len() <= MOST_DESCRIPTION,
        format!("{} characters", description.len()),
    );
    proof.claim(
        "the skill stays short enough to load into every session",
        skill.lines().count() <= MOST_LINES,
        format!("{} lines", skill.lines().count()),
    );
}

fn example_lines(skill: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut inside = false;
    for line in skill.lines() {
        if line.starts_with("```") {
            inside = line == "```bash";
            continue;
        }
        if inside && line.starts_with("neomem ") {
            lines.push(line.to_owned());
        }
    }
    lines
}

fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for character in line.chars() {
        match character {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            other => word.push(other),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn run_examples(neomem: &IsolatedNeomem, skill: &str) -> anyhow::Result<Vec<Example>> {
    let mut last_intake = String::new();
    let mut examples = Vec::new();
    for line in example_lines(skill) {
        let filled = line.replace("<fact-id>", SHIP_DATE).replace("<intake-id>", &last_intake);
        let mut arguments = words(&filled);
        arguments.remove(0);
        arguments.push("--json".to_owned());
        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = neomem.neomem(&borrowed)?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if let Some(intake) = support::first_real_id(&stdout).filter(|id| id.starts_with("int_")) {
            last_intake = intake;
        }
        examples.push(Example {
            line,
            ran: output.status.success(),
            json: serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
            output: stdout,
        });
    }
    Ok(examples)
}

fn every_example_runs(proof: &mut ProofRun, examples: &[Example]) {
    let failed: Vec<&str> = examples
        .iter()
        .filter(|example| !example.ran || !example.json)
        .map(|example| example.line.as_str())
        .collect();
    proof.claim(
        "every example in the skill runs against a live daemon and answers in JSON with --json",
        examples.len() >= AGENT_COMMANDS.len() && failed.is_empty(),
        format!("{} examples, failed: {failed:?}", examples.len()),
    );
}

fn taught_commands(skill: &str) -> BTreeSet<String> {
    skill
        .match_indices("neomem ")
        .filter_map(|(at, _)| {
            let command: String =
                skill[at + "neomem ".len()..].chars().take_while(char::is_ascii_lowercase).collect();
            (!command.is_empty()).then_some(command)
        })
        .collect()
}

fn listed_commands(help: &str) -> Vec<(String, bool)> {
    help.lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?.to_owned();
            Some((name, parts.next().is_some()))
        })
        .collect()
}

fn every_agent_command_is_taught(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    skill: &str,
) -> anyhow::Result<()> {
    let taught = taught_commands(skill);
    let help = String::from_utf8_lossy(&neomem.neomem(&["--help"])?.stdout).into_owned();
    let listed = listed_commands(&help);
    let untaught: Vec<&&str> = AGENT_COMMANDS.iter().filter(|command| !taught.contains(**command)).collect();
    proof.claim(
        "the skill teaches every command an agent should call",
        untaught.is_empty(),
        format!("not taught: {untaught:?}"),
    );
    let unclassified: Vec<&str> = listed
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| !AGENT_COMMANDS.contains(name) && !OPERATOR_COMMANDS.contains(name))
        .collect();
    proof.claim(
        "every command neomem lists is either taught to agents or kept for operators, so a new command cannot slip past the skill",
        unclassified.is_empty() && !listed.is_empty(),
        format!("add these to the skill or to OPERATOR_COMMANDS: {unclassified:?}"),
    );
    let undescribed: Vec<&str> = listed
        .iter()
        .filter(|(name, described)| !described && name != "help")
        .map(|(name, _)| name.as_str())
        .collect();
    proof.claim(
        "neomem --help describes every command in one line",
        undescribed.is_empty(),
        format!("no description: {undescribed:?}"),
    );
    Ok(())
}
