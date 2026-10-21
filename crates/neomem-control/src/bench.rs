use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use neomem_core::ModelRole;
use neomem_proto::{Client, Request, Response, SearchQuery};
use serde::{Deserialize, Serialize};

use crate::binaries::Binaries;
use crate::gpu_build;
use crate::isolated::{IsolatedNeomem, Models};
use crate::model_files::{self, ModelFile};
use crate::repo;
use crate::scenarios::models_embed::distinct_queries;
use crate::seed;
use crate::timing::{self, Percentiles};

const REGRESSION_TOLERANCE: f64 = 1.05;
const NOISE_FLOOR_MS: f64 = 0.05;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub samples: usize,
    pub budget_ms: Option<f64>,
}

impl BenchResult {
    fn from(name: &str, samples: usize, budget_ms: Option<f64>, latency: Percentiles) -> Self {
        Self {
            name: name.to_owned(),
            p50_ms: to_microseconds(latency.p50_ms),
            p95_ms: to_microseconds(latency.p95_ms),
            max_ms: to_microseconds(latency.max_ms),
            samples,
            budget_ms,
        }
    }

    pub fn within_budget(&self) -> bool {
        self.budget_ms.is_none_or(|budget| self.p95_ms <= budget)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchReport {
    pub code_version: String,
    pub results: Vec<BenchResult>,
}

pub fn run(save_baseline: bool) -> anyhow::Result<bool> {
    let root = repo::root()?;
    let binaries = Binaries::build_release(&root)?;
    let report =
        BenchReport { code_version: crate::prove::code_version(&root), results: latency_benches(&binaries)? };
    let baseline_path = root.join("benches/baseline.json");
    let baseline = read_report(&baseline_path);
    let mut healthy = true;
    for result in &report.results {
        let previous =
            baseline.as_ref().and_then(|base| base.results.iter().find(|old| old.name == result.name));
        healthy &= print_result(result, previous);
    }
    if save_baseline {
        fs::create_dir_all(root.join("benches"))?;
        fs::write(&baseline_path, serde_json::to_string_pretty(&report)? + "\n")?;
        println!("baseline saved to benches/baseline.json");
    }
    Ok(healthy)
}

fn latency_benches(binaries: &Binaries) -> anyhow::Result<Vec<BenchResult>> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let mut client = neomem.client()?;
    let socket = timing::measure(2000, || {
        client.call(Request::Status)?;
        Ok(())
    })?;
    let status = timing::measure(200, || {
        neomem.neomem(&["status"])?;
        Ok(())
    })?;
    let add = timing::measure(200, || {
        neomem.neomem(&["add", "bench note about the Edge release", "--source", "bench"])?;
        Ok(())
    })?;
    let mut results = vec![
        BenchResult::from("socket_round_trip", 2000, Some(0.5), socket),
        BenchResult::from("client_status_end_to_end", 200, Some(5.0), status),
        BenchResult::from("client_add_end_to_end", 200, Some(50.0), add),
    ];
    results.extend(search_benches(binaries)?);
    results.extend(recall_benches(binaries)?);
    Ok(results)
}

fn search_benches(binaries: &Binaries) -> anyhow::Result<Vec<BenchResult>> {
    let root = repo::root()?;
    model_files::ensure(&repo::shared_models_dir(&root), model_files::ALL)?;
    let neomem = IsolatedNeomem::start_with(binaries, Models::Real)?;
    seed::copy_vault_fixture("workspace", neomem.home().root())?;
    wait_for_facts(&neomem, 13)?;
    let queries = distinct_queries();
    let mut asked = queries.iter().cycle();
    let mut client = neomem.client()?;
    let socket = timing::measure(60, || {
        let text = asked.next().map_or("Edge", String::as_str);
        client.call(Request::Search(SearchQuery {
            text: format!("{text} search"),
            ..SearchQuery::default()
        }))?;
        Ok(())
    })?;
    let mut asked = queries.iter().cycle();
    let end_to_end = timing::measure(60, || {
        let text = asked.next().map_or("Edge", String::as_str);
        neomem.neomem(&["search", &format!("{text} end to end")])?;
        Ok(())
    })?;
    let mut mcp = crate::scenarios::mcp::StdioClient::open(&neomem)?;
    mcp.call("initialize", serde_json::json!({"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"bench","version":"1"}}))?;
    let mut asked = queries.iter().cycle();
    let mcp_latency = timing::measure(60, || {
        let text = asked.next().map_or("Edge", String::as_str);
        let response = mcp.call(
            "tools/call",
            serde_json::json!({"name":"search","arguments":{"text":format!("{text} MCP")}}),
        )?;
        anyhow::ensure!(
            response["result"]["isError"] != true && response["error"].is_null(),
            "MCP search failed: {response}"
        );
        Ok(())
    })?;
    Ok(vec![
        BenchResult::from("mcp_search_stdio_real_embedder", 60, Some(30.0), mcp_latency),
        BenchResult::from("search_socket_real_embedder", 60, Some(30.0), socket),
        BenchResult::from("search_end_to_end_real_embedder", 60, Some(30.0), end_to_end),
    ])
}

fn wait_for_facts(neomem: &IsolatedNeomem, facts: usize) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while std::time::Instant::now() < deadline {
        if let Response::Status(status) = neomem.client()?.call(Request::Status)?
            && status.index.is_some_and(|index| index.facts >= facts && index.vectors >= facts)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    anyhow::bail!("the bench vault was not indexed in time")
}

fn to_microseconds(millis: f64) -> f64 {
    (millis * 1000.0).round() / 1000.0
}

fn read_report(path: &Path) -> Option<BenchReport> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).context("parsing the baseline").ok()
}

fn print_result(result: &BenchResult, previous: Option<&BenchResult>) -> bool {
    let budget = result.budget_ms.map_or_else(String::new, |budget| {
        let verdict = if result.within_budget() { "within" } else { "OVER" };
        format!(" {verdict} budget {budget} ms")
    });
    let (change, regressed) = previous.map_or((String::new(), false), |old| {
        let ratio = result.p50_ms / old.p50_ms.max(f64::EPSILON);
        let regressed = ratio > REGRESSION_TOLERANCE && result.p50_ms - old.p50_ms > NOISE_FLOOR_MS;
        let flag = if regressed { " REGRESSED" } else { "" };
        (format!(" ({:+.1}% vs baseline{flag})", (ratio - 1.0) * 100.0), regressed)
    });
    println!(
        "{:<28} p50 {:>8.3} ms  p95 {:>8.3} ms  max {:>8.3} ms{budget}{change}",
        result.name, result.p50_ms, result.p95_ms, result.max_ms
    );
    result.within_budget() && !regressed
}

const EMBED_BUDGET_MS: f64 = 20.0;
const LOCAL_JUDGE_BUDGET_MS: f64 = 300.0;
const BENCH_SITUATION: &str = "The release build failed again with the same code signing error as last week";

pub fn models() -> anyhow::Result<bool> {
    let root = repo::root()?;
    let binaries = Binaries::build_release(&root)?;
    model_files::ensure(&repo::shared_models_dir(&root), model_files::ALL)?;
    println!("llama.cpp backend: {}", gpu_build::name(gpu_build::choose()?.as_ref()));
    let mut results = Vec::new();
    for file in [model_files::EMBEDDINGGEMMA, model_files::QWEN3_EMBEDDING] {
        results.extend(embedder_benches(&binaries, file)?);
    }
    results.extend(local_judge_benches(&binaries)?);
    let mut healthy = true;
    for result in &results {
        healthy &= print_result(result, None);
    }
    Ok(healthy)
}

fn model_test(client: &mut Client, role: ModelRole, inputs: Vec<String>) -> anyhow::Result<Duration> {
    let request = Request::ModelsTest { role, source: None, structured: false, inputs };
    match client.call(request)? {
        Response::ModelTested(report) => Ok(Duration::from_secs_f64(report.latency_ms / 1000.0)),
        other => bail!("{role:?} did not answer: {other:?}"),
    }
}

fn embedder_benches(binaries: &Binaries, file: ModelFile) -> anyhow::Result<Vec<BenchResult>> {
    let mut neomem = IsolatedNeomem::prepared_with(binaries, Models::Real)?;
    let config = fs::read_to_string(neomem.home().config_path())?;
    fs::write(neomem.home().config_path(), config.replace(model_files::EMBEDDINGGEMMA.name, file.name))?;
    neomem.start_daemon()?;
    let mut client = neomem.client()?;
    model_test(&mut client, ModelRole::Embedder, vec!["warm up".to_owned()])?;
    let queries = distinct_queries();
    let mut single = queries
        .iter()
        .map(|query| model_test(&mut client, ModelRole::Embedder, vec![query.clone()]))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut batches = (0..10)
        .map(|round| {
            let inputs =
                queries.iter().cycle().skip(round).take(17).map(|text| format!("{text} batch {round}"));
            model_test(&mut client, ModelRole::Embedder, inputs.collect())
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let name = file.name.trim_end_matches(".gguf");
    Ok(vec![
        BenchResult::from(
            &format!("{name} query"),
            single.len(),
            Some(EMBED_BUDGET_MS),
            timing::percentiles(&mut single),
        ),
        BenchResult::from(
            &format!("{name} +16 docs"),
            batches.len(),
            None,
            timing::percentiles(&mut batches),
        ),
    ])
}

fn local_judge_benches(binaries: &Binaries) -> anyhow::Result<Vec<BenchResult>> {
    let mut neomem = IsolatedNeomem::prepared(binaries)?;
    neomem.remove_the_fake_jev_key()?;
    neomem.start_daemon()?;
    let mut client = neomem.client()?;
    model_test(&mut client, ModelRole::Judge, Vec::new())?;
    let mut decide = (0..20)
        .map(|_| model_test(&mut client, ModelRole::Judge, Vec::new()))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let candidates = distinct_queries().into_iter().take(15);
    let inputs: Vec<String> = std::iter::once(BENCH_SITUATION.to_owned()).chain(candidates).collect();
    let mut score = (0..10)
        .map(|_| model_test(&mut client, ModelRole::Judge, inputs.clone()))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let name = model_files::QWEN3_RERANKER.name.trim_end_matches(".gguf");
    Ok(vec![
        BenchResult::from(
            &format!("{name} decide 2"),
            decide.len(),
            Some(LOCAL_JUDGE_BUDGET_MS),
            timing::percentiles(&mut decide),
        ),
        BenchResult::from(
            &format!("{name} score 15"),
            score.len(),
            Some(LOCAL_JUDGE_BUDGET_MS),
            timing::percentiles(&mut score),
        ),
    ])
}

fn recall_benches(binaries: &Binaries) -> anyhow::Result<Vec<BenchResult>> {
    use crate::scenarios::recall_support::{planner, rule};
    use neomem_proto::RecallQuery;
    use neomem_testkit::FakeRules;
    let neomem = IsolatedNeomem::start(binaries)?;
    seed::copy_vault_fixture("workspace", neomem.home().root())?;
    wait_for_facts(&neomem, 13)?;
    neomem.add_fake_rules(&FakeRules {
        judge: vec![rule("long-term memory", None, 0.49)],
        ..FakeRules::default()
    })?;
    let mut client = neomem.client()?;
    let mut serial = 0;
    let mut measure = || {
        serial += 1;
        match client.call(Request::Recall(RecallQuery {
            agent: "codex".to_owned(),
            session: format!("bench-{serial}"),
            turn: None,
            slice: "Edge release failed".to_owned(),
            message: "Edge signing error again".to_owned(),
            error: None,
            project: None,
        }))? {
            Response::Recalled(_) => Ok(()),
            other => bail!("recall answered {other:?}"),
        }
    };
    let skipped = timing::measure(20, &mut measure)?;
    neomem.add_fake_rules(&FakeRules {
        judge: vec![rule("long-term memory", None, 0.9), rule("this memory", None, 0.9)],
        llm: vec![planner(0, 0)],
        ..FakeRules::default()
    })?;
    let full = timing::measure(20, measure)?;
    Ok(vec![
        BenchResult::from("recall_gated_out_fake", 20, Some(600.0), skipped),
        BenchResult::from("recall_full_fake", 20, Some(3500.0), full),
    ])
}
