use std::process::ExitCode;

use neomem_proto::{Response, StatusReport};

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

fn print_human(response: &Response) {
    match response {
        Response::Status(report) => print_status(report),
        Response::ShuttingDown => println!("neomemd is shutting down"),
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
