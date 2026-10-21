use std::path::Path;
use std::process::ExitCode;

use crate::time_argument;

use neomem_proto::{
    EntityView, IndexReport, RawHit, RebuildReport, Response, SearchResults, ShownMemory, StatusReport,
};

pub fn print(response: &Response, json: bool) -> ExitCode {
    if json {
        println!("{}", serde_json::to_string(response).unwrap_or_default());
    } else {
        print_human(response);
    }
    match response {
        Response::Failed(_) | Response::VersionMismatch { .. } => ExitCode::FAILURE,
        _ => ExitCode::SUCCESS,
    }
}

pub fn print_spooled(path: &Path, json: bool) -> ExitCode {
    if json {
        println!("{}", serde_json::json!({ "spooled": path }));
    } else {
        println!("spooled: neomemd is down; the request waits in {}", path.display());
    }
    ExitCode::SUCCESS
}

fn print_human(response: &Response) {
    match response {
        Response::Captured => {}
        Response::Status(report) => print_status(report),
        Response::ShuttingDown => println!("neomemd is shutting down"),
        Response::Added(receipt) => println!("{}", receipt.id),
        Response::RawHits(hits) => print_raw_hits(hits),
        Response::Corrected(receipt) => println!("{} is marked as logged incorrectly", receipt.intake),
        Response::Rebuilt(report) => print_rebuilt(report),
        Response::Reextracting { queued } => println!("queued {queued} intake records for extraction"),
        Response::SearchResults(results) => print_search(results),
        Response::DriftChecked(report) => println!(
            "drift check: {} differences ({} added, {} changed, {} files missing, {} unreadable) in {} ms",
            report.differences,
            report.added,
            report.changed,
            report.missing_files,
            report.unreadable,
            report.took_ms
        ),
        Response::ClockSet { now } => println!("clock set to {}", time_argument::display(*now)),
        Response::Resolved(found) => {
            for entity in found {
                println!("{}  {} ({})", entity.id, entity.name, entity.kind);
            }
        }
        Response::Entity(view) => print_entity(view),
        Response::Shown(shown) => print_shown(shown),
        Response::ModelRoster(roster) => {
            for assignment in roster {
                println!("{:<13} {}", assignment.role.name(), assignment.assignment);
            }
        }
        Response::ModelTested(report) => {
            let first_text = report
                .first_text_ms
                .map(|first| format!(", first text after {first:.1} ms"))
                .unwrap_or_default();
            println!(
                "{} {} answered in {:.1} ms{first_text}: {}",
                report.role.name(),
                report.model,
                report.latency_ms,
                report.answer
            );
            for (index, score) in report.scores.iter().enumerate() {
                println!("  input {}: {score:.4}", index + 2);
            }
        }
        Response::LoginOpened { url, .. } => println!("{url}"),
        Response::LoggedIn { provider, account } => println!("signed in to {provider} as {account}"),
        Response::VersionMismatch { daemon, client } => {
            eprintln!("neomemd speaks protocol {daemon}, this neomem speaks {client}");
        }
        Response::Failed(failure) => eprintln!("neomem: {}", failure.message),
    }
}

fn print_status(report: &StatusReport) {
    println!("neomemd {} (protocol {})", report.daemon_version, report.protocol);
    println!("home     {}", report.home);
    println!("pid      {}", report.pid);
    println!("uptime   {} s", report.uptime_ms / 1000);
    if let Some(index) = &report.index {
        print_index(index);
    }
}

fn print_raw_hits(hits: &[RawHit]) {
    if hits.is_empty() {
        println!("no intake records match");
    }
    for hit in hits {
        let corrected = if hit.corrected { "  [corrected]" } else { "" };
        println!("{}  {}  {}{corrected}", time_argument::display(hit.occurred.start), hit.source, hit.id);
        println!("  {}", hit.content.replace('\n', "\n  "));
    }
}

fn print_rebuilt(report: &RebuildReport) {
    println!(
        "rebuilt index.db in {} ms ({} unreadable files skipped)",
        report.took_ms, report.unreadable_files
    );
    print_index(&report.index);
}

fn print_index(index: &IndexReport) {
    println!(
        "index    {} facts, {} entities, {} links, {} names, {} vectors ({})",
        index.facts,
        index.entities,
        index.links,
        index.names,
        index.vectors,
        index.model.as_deref().unwrap_or("no embedder")
    );
}

fn print_entity(view: &EntityView) {
    let entity = &view.entity;
    println!("{}  {} ({})", entity.id(), entity.name(), entity.kind());
    if !entity.aliases().is_empty() {
        println!("also     {}", entity.aliases().join(", "));
    }
    if !entity.description().is_empty() {
        println!("about    {}", entity.description());
    }
    for fact in &view.facts {
        println!(
            "  {}  {}  (valid since {})",
            fact.id(),
            fact.content(),
            time_argument::display(fact.valid_at())
        );
    }
    for linked in &view.linked {
        println!("  {} {}  {}", linked.kind, linked.entity.name(), linked.entity.id());
    }
}

fn print_shown(shown: &ShownMemory) {
    print!("{}", shown.text);
    for source in &shown.sources {
        let corrected = if source.corrected { "  [corrected]" } else { "" };
        match &source.record {
            Some(record) => {
                println!("source {} ({}){corrected}: {}", source.id, record.source, record.content);
            }
            None => println!("source {} is not in the intake log", source.id),
        }
    }
}

fn print_search(results: &SearchResults) {
    if results.hits.is_empty() {
        println!("no memories match");
    }
    for (rank, hit) in results.hits.iter().enumerate() {
        let fact = &hit.fact;
        let conflict = if hit.conflict { "  [conflict]" } else { "" };
        let ended = fact
            .invalid_at()
            .map(|ended| format!(", invalid since {}", time_argument::display(ended)))
            .unwrap_or_default();
        println!("{}. {}{conflict}", rank + 1, fact.content());
        println!(
            "   {}  valid since {}{ended}  score {:.3}",
            fact.id(),
            time_argument::display(fact.valid_at()),
            hit.score
        );
        if !hit.replaced.is_empty() {
            let replaced: Vec<String> = hit.replaced.iter().map(ToString::to_string).collect();
            println!("   replaces {}", replaced.join(", "));
        }
        if !hit.entities.is_empty() {
            let names: Vec<&str> = hit.entities.iter().map(|entity| entity.name.as_str()).collect();
            println!("   about {}", names.join(", "));
        }
        for linked in &hit.linked {
            println!("   {} {}  ({})", linked.kind, linked.fact.content(), linked.fact.id());
        }
    }
}
