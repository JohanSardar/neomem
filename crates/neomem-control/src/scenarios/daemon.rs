use std::process::Command;
use std::time::{Duration, Instant};

use neomem_proto::{Envelope, PROTOCOL_VERSION, Request, Response};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::timing;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    status_answers_over_the_socket(proof, &neomem)?;
    socket_round_trips_are_fast(proof, &neomem)?;
    the_thin_client_reports_status(proof, &neomem)?;
    http_gives_the_same_answer(proof, &neomem)?;
    other_protocol_versions_are_told_so(proof, &neomem)?;
    a_second_daemon_steps_aside(proof, &neomem)?;
    a_stopped_daemon_starts_on_demand(proof, &mut neomem)?;
    proof.attach_logs(&neomem.logs());
    long_homes_still_get_a_socket(proof, binaries)?;
    Ok(())
}

fn status_answers_over_the_socket(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let response = neomem.call(proof, Request::Status)?;
    let expected_home = neomem.home().root().display().to_string();
    let holds = matches!(&response, Response::Status(report)
        if report.protocol == PROTOCOL_VERSION && report.home == expected_home);
    proof.claim(
        "status over the socket names the current protocol and the isolated home",
        holds,
        format!("{response:?}"),
    );
    Ok(())
}

fn socket_round_trips_are_fast(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let mut client = neomem.client()?;
    let latency = timing::measure(200, || {
        client.call(Request::Status)?;
        Ok(())
    })?;
    proof.claim("a socket request and reply takes under 0.5 ms at the median", latency.p50_ms < 0.5, latency);
    Ok(())
}

fn the_thin_client_reports_status(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let output = neomem.neomem(&["status", "--json"])?;
    let parsed: Response = serde_json::from_slice(&output.stdout)?;
    let holds = output.status.success() && matches!(parsed, Response::Status(_));
    proof.claim(
        "neomem status --json prints the daemon's status",
        holds,
        String::from_utf8_lossy(&output.stdout),
    );
    let latency = timing::measure(20, || {
        neomem.neomem(&["status"])?;
        Ok(())
    })?;
    proof.claim(
        "neomem status answers end to end within 25 ms at the median (the 2-5 ms budget is checked by neomemctl bench on release builds)",
        latency.p50_ms < 25.0,
        latency,
    );
    Ok(())
}

fn http_gives_the_same_answer(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let url = format!("http://{}/v1/request", neomem.http_address()?);
    let response: Response =
        ureq::post(&url).send_json(Envelope::current(Request::Status))?.body_mut().read_json()?;
    let holds = matches!(&response, Response::Status(report) if report.protocol == PROTOCOL_VERSION);
    proof.claim("HTTP POST /v1/request answers status with the same handler", holds, format!("{response:?}"));
    Ok(())
}

fn other_protocol_versions_are_told_so(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let stale = Envelope { protocol: PROTOCOL_VERSION + 1, request: Request::Status };
    let response = neomem.send(proof, &stale)?;
    let holds =
        response == Response::VersionMismatch { daemon: PROTOCOL_VERSION, client: PROTOCOL_VERSION + 1 };
    proof.claim("a client on another protocol version gets version_mismatch", holds, format!("{response:?}"));
    Ok(())
}

fn a_second_daemon_steps_aside(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let second =
        Command::new(&neomem.binaries().neomemd).env("NEOMEM_HOME", neomem.home().root()).output()?;
    let first_still_serves = matches!(neomem.call(proof, Request::Status)?, Response::Status(_));
    proof.claim(
        "a second neomemd on the same home exits cleanly and the first keeps serving",
        second.status.success() && first_still_serves,
        format!("second exit {}", second.status),
    );
    Ok(())
}

fn a_stopped_daemon_starts_on_demand(
    proof: &mut ProofRun,
    neomem: &mut IsolatedNeomem,
) -> anyhow::Result<()> {
    neomem.stop_daemon()?;
    let started = Instant::now();
    let output = neomem.neomem(&["status", "--json"])?;
    let elapsed = started.elapsed();
    proof.claim(
        "neomem status starts a stopped daemon and answers within 2 seconds",
        output.status.success() && elapsed < Duration::from_secs(2),
        format!("{elapsed:?}"),
    );
    Ok(())
}

fn long_homes_still_get_a_socket(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let base = tempfile::Builder::new().prefix("neomemctl-long-").tempdir()?;
    let deep = base.path().join("a-very-long-directory-name-for-the-socket-test".repeat(3));
    let neomem = IsolatedNeomem::start_at(&deep, binaries)?;
    let socket = neomem.home().socket_path();
    let answers = matches!(neomem.call(proof, Request::Status)?, Response::Status(_));
    proof.claim(
        "a home too long for a Unix socket path still gets a working socket elsewhere",
        answers && !socket.starts_with(&deep),
        socket.display(),
    );
    Ok(())
}
