use std::path::Path;
use std::process::Command;

use anyhow::bail;

use crate::binaries::Binaries;
use crate::graph::{Feature, FeatureGraph};
use crate::proof::ProofRun;
use crate::{changes, repo, scenarios};

#[derive(Debug)]
pub enum Selection {
    One(String),
    Changed,
    All,
}

pub fn run(selection: &Selection) -> anyhow::Result<bool> {
    let root = repo::root()?;
    let graph_path = repo::feature_graph_path(&root);
    let mut graph = FeatureGraph::load(&graph_path)?;
    let chosen: Vec<Feature> = choose(&graph.features, selection, &root)?.into_iter().cloned().collect();
    if chosen.is_empty() {
        eprintln!("prove: no features selected");
        return Ok(true);
    }
    let code_version = code_version(&root);
    let binaries = Binaries::build_fresh(&root)?;
    let mut every_proof_passed = true;
    for feature in &chosen {
        let passed = prove_one(feature, &root, &code_version, &binaries, &mut graph)?;
        every_proof_passed &= passed;
    }
    graph.save(&graph_path)?;
    Ok(every_proof_passed)
}

fn prove_one(
    feature: &Feature,
    root: &Path,
    code_version: &str,
    binaries: &Binaries,
    graph: &mut FeatureGraph,
) -> anyhow::Result<bool> {
    let mut proof_run = ProofRun::new(&feature.id, &feature.proof);
    let outcome = match scenarios::find(&feature.proof) {
        Some(scenario) => (scenario.run)(&mut proof_run, binaries),
        None => Err(anyhow::anyhow!("no proof scenario named {} exists", feature.proof)),
    };
    proof_run.print_claims();
    let bundle = proof_run.finish(outcome, &root.join("proofs"), code_version)?;
    let relative = bundle.path.strip_prefix(root).unwrap_or(&bundle.path).display().to_string();
    let verdict = if bundle.passed { "PROVEN" } else { "FAILED" };
    eprintln!(
        "prove {}: {verdict} ({}/{} checks) -> {relative}",
        feature.id, bundle.checks_passed, bundle.checks_total
    );
    if let Some(failure) = &bundle.failure {
        eprintln!("  failure: {failure}");
    }
    graph.record_proof(&feature.id, bundle.passed, relative);
    Ok(bundle.passed)
}

fn code_version(root: &Path) -> String {
    let git = |arguments: &[&str]| {
        Command::new("git")
            .args(arguments)
            .current_dir(root)
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let head = git(&["rev-parse", "--short=12", "HEAD"]);
    let dirty = !git(&["status", "--porcelain", "--untracked-files=no"]).is_empty();
    if dirty { format!("{head}-dirty") } else { head }
}

fn choose<'graph>(
    features: &'graph [Feature],
    selection: &Selection,
    root: &Path,
) -> anyhow::Result<Vec<&'graph Feature>> {
    match selection {
        Selection::All => Ok(features.iter().collect()),
        Selection::One(id) => match features.iter().find(|feature| &feature.id == id) {
            Some(feature) => Ok(vec![feature]),
            None => bail!("feature {id} is not in the feature graph"),
        },
        Selection::Changed => {
            let changed = changes::changed_files(root)?;
            Ok(features
                .iter()
                .filter(|feature| feature.status != crate::graph::Status::Planned)
                .filter(|feature| {
                    let crate_dir = format!("crates/{}/", feature.crate_name());
                    changed.iter().any(|file| file.starts_with(&crate_dir))
                })
                .collect())
        }
    }
}
