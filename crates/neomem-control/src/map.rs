use crate::graph::{FeatureGraph, Status};
use crate::repo;

#[derive(Debug, Clone, Copy)]
pub enum View {
    Status,
    Next,
}

pub fn run(view: View) -> anyhow::Result<bool> {
    let graph = FeatureGraph::load(&repo::feature_graph_path(&repo::root()?))?;
    match view {
        View::Status => print_status(&graph),
        View::Next => print_next(&graph),
    }
    Ok(true)
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
