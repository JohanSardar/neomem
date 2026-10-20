use crate::graph::{FeatureGraph, Status};
use std::collections::BTreeSet;
use std::path::Path;

use crate::{repo, scenarios, surfaces};

#[derive(Debug, Clone, Copy)]
pub enum View {
    Status,
    Next,
    Check,
}

pub fn run(view: View) -> anyhow::Result<bool> {
    let root = repo::root()?;
    let graph = FeatureGraph::load(&repo::feature_graph_path(&root))?;
    match view {
        View::Status => print_status(&graph),
        View::Next => print_next(&graph),
        View::Check => return report_drift(&graph, &root),
    }
    Ok(true)
}

fn report_drift(graph: &FeatureGraph, root: &Path) -> anyhow::Result<bool> {
    let drift = drift_between_graph_and_code(graph, root)?;
    for problem in &drift {
        eprintln!("[feature-graph] {problem}");
    }
    Ok(drift.is_empty())
}

pub fn drift_between_graph_and_code(graph: &FeatureGraph, root: &Path) -> anyhow::Result<Vec<String>> {
    let code = surfaces::in_code(root)?;
    let mut drift = Vec::new();
    for feature in graph.features.iter().filter(|feature| feature.status != Status::Planned) {
        if scenarios::find(&feature.proof).is_none() {
            drift.push(format!(
                "{} is {} but its proof scenario {} does not exist",
                feature.id,
                feature.status.label(),
                feature.proof
            ));
        }
        for surface in feature.surfaces().iter().filter(|surface| surfaces::is_checked_surface(surface)) {
            if !surfaces::graph_surface_exists(surface, &code) {
                drift.push(format!("{} lists `{surface}`, which the code does not have", feature.id));
            }
        }
    }
    let listed: BTreeSet<&str> =
        graph.features.iter().flat_map(crate::graph::Feature::surfaces).map(String::as_str).collect();
    for surface in &code {
        if !surfaces::code_surface_is_listed(surface, &listed) {
            drift.push(format!("the code has `{surface}`, which no feature lists"));
        }
    }
    Ok(drift)
}
fn print_status(graph: &FeatureGraph) {
    for feature in &graph.features {
        let marker = match feature.status {
            Status::Proven => "✓",
            _ if graph.is_unblocked(feature) => "→",
            _ => " ",
        };
        let blocked =
            feature.blocked.as_deref().map(|reason| format!("  [blocked: {reason}]")).unwrap_or_default();
        println!("{marker} {:<16} {:<9}{blocked}", feature.id, feature.status.label());
    }
    let proven = graph.features.iter().filter(|feature| feature.status == Status::Proven).count();
    println!("{proven}/{} proven", graph.features.len());
}

fn print_next(graph: &FeatureGraph) {
    match graph.next_to_build() {
        Some(feature) => {
            println!("{}", feature.id);
            println!("  does:       {}", feature.does);
            println!("  module:     {}", feature.module);
            println!("  reached:    {}", feature.surfaces().join(", "));
            println!("  depends on: {}", feature.dependencies().join(", "));
            println!("  proof:      neomemctl prove {}", feature.id);
        }
        None => println!("every unblocked feature is proven"),
    }
}
