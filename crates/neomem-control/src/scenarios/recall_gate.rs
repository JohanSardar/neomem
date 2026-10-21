use super::recall_support::{self as support, PROMPT};
use crate::binaries::Binaries;
use crate::proof::ProofRun;
use neomem_testkit::FakeRules;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = support::setup(proof, binaries)?;
    thresholds(proof, &neomem)?;
    error_only_turn(proof, &neomem)?;
    switches(proof, &neomem)?;
    failures(proof, &mut neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn thresholds(proof: &mut ProofRun, neomem: &crate::isolated::IsolatedNeomem) -> anyhow::Result<()> {
    for probability in [0.49, 0.5] {
        neomem.add_fake_rules(&FakeRules {
            judge: vec![support::rule("long-term memory", None, probability)],
            ..FakeRules::default()
        })?;
        let before = neomem.fake_model_calls()?.len();
        let (memories, elapsed) = support::recall(
            neomem,
            proof,
            if probability < 0.5 { "gate-skipped" } else { "gate-open" },
            PROMPT,
        )?;
        let calls = neomem.fake_model_calls()?;
        let calls = &calls[before..];
        let (gate, queries, _, _, _) = support::last_log(neomem)?;
        let holds = if probability < 0.5 {
            memories.is_empty() && support::judge_calls(calls) == 1 && calls.len() == 1 && queries == 0
        } else {
            !memories.is_empty() && support::judge_calls(calls) == 2 && queries >= 3
        };
        proof.claim(
            &format!(
                "gate at P={probability} {}",
                if probability < 0.5 { "stops after one noul call" } else { "opens the pipeline" }
            ),
            holds && gate == Some(probability),
            format!(
                "{} memories, {} calls, {:.2} ms",
                memories.len(),
                calls.len(),
                elapsed.as_secs_f64() * 1000.0
            ),
        );
    }
    neomem.call(
        proof,
        neomem_proto::Request::Search(neomem_proto::SearchQuery {
            text: PROMPT.to_owned(),
            recall_turn: Some(neomem_proto::RecallTurn {
                agent: "codex".to_owned(),
                session: "gate-skipped".to_owned(),
                turn: "gate-skipped".to_owned(),
            }),
            ..neomem_proto::SearchQuery::default()
        }),
    )?;
    let labels: i64 =
        support::db(neomem)?.query_row("SELECT count(*) FROM recall_search_labels", [], |row| row.get(0))?;
    proof.claim(
        "an agent search on a skipped turn records a free gate training label without content",
        labels == 1,
        labels,
    );
    Ok(())
}

fn switches(proof: &mut ProofRun, neomem: &crate::isolated::IsolatedNeomem) -> anyhow::Result<()> {
    let output = neomem.neomem(&["recall", "--agent", "codex", "--message", PROMPT])?;
    proof.claim(
        "recall is off by default and prints nothing",
        output.status.success() && output.stdout.is_empty(),
        "",
    );
    for arguments in
        [vec!["recall", "on", "--agent", "codex"], vec!["recall", "off", "--project", "/proof/project"]]
    {
        proof.claim(
            "recall switches persist without model calls",
            neomem.neomem(&arguments)?.status.success(),
            "",
        );
    }
    let config = std::fs::read_to_string(neomem.home().config_path())?;
    proof.claim(
        "agent and project switches are in config.toml",
        config.contains("codex = true") && config.contains("\"/proof/project\" = false"),
        "",
    );
    let output = neomem.neomem(&["recall", "--agent", "codex", "--session", "cli", "--message", PROMPT])?;
    proof.claim(
        "enabled recall CLI prints approved memories and fct IDs",
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("fct_"),
        String::from_utf8_lossy(&output.stderr),
    );
    neomem.neomem(&["recall", "off", "--project", "."])?;
    let output = neomem.neomem(&[
        "recall",
        "--agent",
        "codex",
        "--session",
        "relative-project",
        "--message",
        PROMPT,
    ])?;
    proof.claim(
        "a relative project switch applies to the current project and overrides the agent switch",
        output.status.success() && output.stdout.is_empty(),
        "relative project scope",
    );

    Ok(())
}

fn failures(proof: &mut ProofRun, neomem: &mut crate::isolated::IsolatedNeomem) -> anyhow::Result<()> {
    neomem.plan_with("fake")?;
    let config = std::fs::read_to_string(neomem.home().config_path())?;
    neomem.add_fake_rules(&FakeRules {
        judge: vec![support::rule("long-term memory", None, 1.1)],
        ..FakeRules::default()
    })?;
    let (memories, elapsed) = support::recall(neomem, proof, "invalid-judge", PROMPT)?;
    proof.claim(
        "an invalid judge probability fails open quickly",
        memories.is_empty() && elapsed.as_millis() < 600,
        format!("{:.1} ms", elapsed.as_secs_f64() * 1000.0),
    );
    neomem.stop_daemon()?;
    let config = config
        .lines()
        .map(|line| {
            if line.starts_with("judge =") {
                "judge = { provider = \"fake_jev\", timeout = \"600ms\" }"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(neomem.home().config_path(), &config)?;
    neomem.start_daemon()?;
    neomem.add_fake_rules(&FakeRules {
        latency_ms: std::collections::BTreeMap::from([("/v1/systemone".to_owned(), 2000)]),
        ..FakeRules::default()
    })?;
    let (memories, elapsed) = support::recall(neomem, proof, "judge-timeout", PROMPT)?;
    proof.claim(
        "a judge timeout fails open quickly",
        memories.is_empty() && elapsed.as_millis() < 1200,
        format!("{:.1} ms", elapsed.as_secs_f64() * 1000.0),
    );
    neomem.stop_daemon()?;
    std::fs::write(
        neomem.home().config_path(),
        config.replace("timeout = \"600ms\"", "timeout = \"5s\"") + "\n[recall]\nhard_timeout = \"150ms\"\n",
    )?;
    neomem.start_daemon()?;
    let (memories, elapsed) = support::recall(neomem, proof, "hard-timeout", PROMPT)?;
    proof.claim(
        "the hard deadline returns nothing without waiting for an unresponsive model",
        memories.is_empty() && elapsed.as_millis() >= 130 && elapsed.as_millis() < 500,
        format!("{:.1} ms", elapsed.as_secs_f64() * 1000.0),
    );
    Ok(())
}

fn error_only_turn(proof: &mut ProofRun, neomem: &crate::isolated::IsolatedNeomem) -> anyhow::Result<()> {
    neomem.add_fake_rules(&FakeRules {
        judge: vec![
            support::rule("long-term memory", None, 0.1),
            support::rule("long-term memory", Some(PROMPT), 0.9),
        ],
        ..FakeRules::default()
    })?;
    let response = neomem.call(
        proof,
        neomem_proto::Request::Recall(neomem_proto::RecallQuery {
            agent: "codex".to_owned(),
            session: "error-only".to_owned(),
            turn: None,
            slice: "The build failed".to_owned(),
            message: String::new(),
            error: Some(PROMPT.to_owned()),
            project: None,
        }),
    )?;
    proof.claim(
        "a new error without a new message is scored by the gate and recalls memories",
        matches!(response, neomem_proto::Response::Recalled(memories) if !memories.is_empty()),
        "error-only turn",
    );
    neomem.add_fake_rules(&FakeRules {
        judge: vec![support::rule("long-term memory", None, 0.9)],
        ..FakeRules::default()
    })?;
    Ok(())
}
