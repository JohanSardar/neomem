use std::path::Path;
use std::process::ExitCode;

use crate::time_argument;

use neomem_proto::{IndexReport, RawHit, RebuildReport, Response, StatusReport};

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
        Response::Status(report) => print_status(report),
        Response::ShuttingDown => println!("neomemd is shutting down"),
        Response::Added(receipt) => println!("{}", receipt.id),
        Response::RawHits(hits) => print_raw_hits(hits),
        Response::Corrected(receipt) => println!("{} is marked as logged incorrectly", receipt.intake),
        Response::Rebuilt(report) => print_rebuilt(report),
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
