use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use anyhow::Context;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Check {
    pub claim: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Exchange {
    pub request: serde_json::Value,
    pub response: serde_json::Value,
    pub elapsed_ms: f64,
}

#[derive(Debug, Serialize)]
struct ProofSummary<'run> {
    feature: &'run str,
    scenario: &'run str,
    passed: bool,
    started_at: String,
    duration_ms: u128,
    code_version: &'run str,
    models: &'run str,
    failure: Option<String>,
    checks: &'run [Check],
}

#[derive(Debug)]
pub struct ProofRun {
    feature: String,
    scenario: String,
    started_at: SystemTime,
    clock: Instant,
    pub models: String,
    checks: Vec<Check>,
    exchanges: Vec<Exchange>,
    logs: String,
}

#[derive(Debug)]
pub struct Bundle {
    pub passed: bool,
    pub path: PathBuf,
    pub checks_passed: usize,
    pub checks_total: usize,
    pub failure: Option<String>,
}

const LOG_LIMIT_BYTES: usize = 256 * 1024;

impl ProofRun {
    pub fn new(feature: &str, scenario: &str) -> Self {
        Self {
            feature: feature.to_owned(),
            scenario: scenario.to_owned(),
            started_at: SystemTime::now(),
            clock: Instant::now(),
            models: "fake".to_owned(),
            checks: Vec::new(),
            exchanges: Vec::new(),
            logs: String::new(),
        }
    }

    pub fn claim(&mut self, claim: &str, holds: bool, detail: impl Display) -> bool {
        self.checks.push(Check { claim: claim.to_owned(), passed: holds, detail: detail.to_string() });
        holds
    }

    pub fn record_exchange(&mut self, exchange: Exchange) {
        self.exchanges.push(exchange);
    }

    pub fn attach_logs(&mut self, logs: &str) {
        let start = logs.len().saturating_sub(LOG_LIMIT_BYTES);
        let start = (start..logs.len()).find(|index| logs.is_char_boundary(*index)).unwrap_or(logs.len());
        self.logs.push_str(&logs[start..]);
    }

    pub fn finish(
        self,
        outcome: anyhow::Result<()>,
        proofs_root: &Path,
        code_version: &str,
    ) -> anyhow::Result<Bundle> {
        let failure = outcome.err().map(|error| format!("{error:#}"));
        let checks_passed = self.checks.iter().filter(|check| check.passed).count();
        let passed = failure.is_none() && !self.checks.is_empty() && checks_passed == self.checks.len();
        let stamp = humantime::format_rfc3339_seconds(self.started_at).to_string().replace(':', "");
        let feature_dir = proofs_root.join(&self.feature);
        let path = feature_dir.join(&stamp);
        fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        let summary = ProofSummary {
            feature: &self.feature,
            scenario: &self.scenario,
            passed,
            started_at: stamp.clone(),
            duration_ms: self.clock.elapsed().as_millis(),
            code_version,
            models: &self.models,
            failure: failure.clone(),
            checks: &self.checks,
        };
        fs::write(path.join("proof.json"), serde_json::to_string_pretty(&summary)?)?;
        let exchanges: Vec<String> =
            self.exchanges.iter().map(serde_json::to_string).collect::<Result<_, _>>()?;
        fs::write(path.join("exchanges.jsonl"), exchanges.join("\n"))?;
        fs::write(path.join("daemon.log"), &self.logs)?;
        remove_older_bundles(&feature_dir, &stamp)?;
        Ok(Bundle { passed, path, checks_passed, checks_total: self.checks.len(), failure })
    }
}

fn remove_older_bundles(feature_dir: &Path, keep: &str) -> anyhow::Result<()> {
    for entry in fs::read_dir(feature_dir)? {
        let entry = entry?;
        if entry.file_name() != keep && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_passes_only_when_it_checked_something_and_every_check_held() {
        let proofs = tempfile::tempdir().unwrap();
        let empty_bundle = ProofRun::new("daemon", "daemon").finish(Ok(()), proofs.path(), "abc").unwrap();
        assert!(!empty_bundle.passed);

        let mut checked = ProofRun::new("daemon", "daemon");
        checked.claim("socket answers", true, "0.3 ms");
        let bundle = checked.finish(Ok(()), proofs.path(), "abc").unwrap();
        assert!(bundle.passed);
        assert!(bundle.path.join("proof.json").exists());
    }

    #[test]
    fn a_failed_claim_fails_the_proof_and_keeps_only_the_newest_bundle() {
        let proofs = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(proofs.path().join("index/2000-01-01T000000Z")).unwrap();
        let mut run = ProofRun::new("index", "index");
        run.claim("rebuild is identical", false, "3 rows differ");
        let bundle = run.finish(Ok(()), proofs.path(), "abc").unwrap();
        assert!(!bundle.passed);
        let bundles = std::fs::read_dir(proofs.path().join("index")).unwrap().count();
        assert_eq!(bundles, 1);
    }
}
