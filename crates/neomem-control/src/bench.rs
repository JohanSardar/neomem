use std::fs;
use std::path::Path;

use anyhow::Context;
use neomem_proto::Request;
use serde::{Deserialize, Serialize};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::repo;
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
    Ok(vec![
        BenchResult::from("socket_round_trip", 2000, Some(0.5), socket),
        BenchResult::from("client_status_end_to_end", 200, Some(5.0), status),
        BenchResult::from("client_add_end_to_end", 200, Some(50.0), add),
    ])
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
