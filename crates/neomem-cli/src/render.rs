use std::path::Path;
use std::process::ExitCode;

use crate::time_argument;

use neomem_proto::{RawHit, Response, StatusReport};

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
        Response::ModelRoster(roster) => {
            for assignment in roster {
                println!("{:<13} {}", assignment.role.name(), assignment.assignment);
            }
        }
        Response::ModelTested(report) => {
            println!(
                "{} {} answered in {:.1} ms: {}",
                report.role.name(),
                report.model,
                report.latency_ms,
                report.answer
            );
        }
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
