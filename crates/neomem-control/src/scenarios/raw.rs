use neomem_core::{EpochMs, IntakeRecord, Source, Span};
use neomem_proto::{RawHit, RawQuery, Request, Response};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::timing;

const DAY: i64 = 86_400_000;
const TUESDAY: i64 = 1_792_540_800_000;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    log_history(&neomem)?;
    keywords_find_the_right_record(proof, &neomem)?;
    dates_and_sources_narrow_the_search(proof, &neomem)?;
    old_records_rank_like_new_ones(proof, &neomem)?;
    the_command_line_prints_the_hits(proof, &neomem)?;
    raw_search_stays_fast_over_thousands_of_records(proof, &neomem)?;
    search_falls_back_to_keywords_when_the_daemon_cannot_start(proof, &mut neomem, binaries)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn record(source: &str, at: i64, content: &str) -> anyhow::Result<IntakeRecord> {
    Ok(IntakeRecord {
        source: Source::new(source)?,
        occurred: Span::instant(EpochMs::from_millis(at)),
        tags: Vec::new(),
        content: content.to_owned(),
        entity_hints: Vec::new(),
        source_ref: None,
        session: None,
    })
}

fn log_history(neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let mut client = neomem.client()?;
    let history = [
        (
            "claude-code",
            TUESDAY + 10 * 3_600_000,
            "The Edge build fails with a code signing error on the release config",
        ),
        ("claude-code", TUESDAY - DAY, "Renamed the paywall component in Edge"),
        ("oura-digest", TUESDAY + 7 * 3_600_000, "Slept 7h 12m, readiness 84"),
        ("codex", TUESDAY + 2 * DAY, "Fixed the signing error by rotating the Edge distribution certificate"),
        ("note", 1_577_836_800_000, "Back in 2020 the signing error came from an expired certificate"),
    ];
    for (source, at, content) in history {
        client.call(Request::Add(record(source, at, content)?))?;
    }
    Ok(())
}

fn search(neomem: &IsolatedNeomem, proof: &mut ProofRun, query: RawQuery) -> anyhow::Result<Vec<RawHit>> {
    match neomem.call(proof, Request::Raw(query))? {
        Response::RawHits(hits) => Ok(hits),
        other => anyhow::bail!("raw search answered {other:?}"),
    }
}

fn query(text: &str) -> RawQuery {
    RawQuery { text: text.to_owned(), from: None, to: None, source: None, limit: 20 }
}

fn keywords_find_the_right_record(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let hits = search(neomem, proof, query("signing error"))?;
    let contents: Vec<&str> = hits.iter().map(|hit| hit.content.as_str()).collect();
    proof.claim(
        "keywords find every record that says them, and only those",
        hits.len() == 3 && contents.iter().all(|content| content.contains("signing error")),
        format!("{contents:?}"),
    );
    let exact = hits
        .iter()
        .any(|hit| hit.content == "Fixed the signing error by rotating the Edge distribution certificate");
    proof.claim("a hit carries the record's exact content", exact, hits.len());
    Ok(())
}

fn dates_and_sources_narrow_the_search(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let tuesday = RawQuery {
        text: String::new(),
        from: Some(EpochMs::from_millis(TUESDAY)),
        to: Some(EpochMs::from_millis(TUESDAY + DAY - 1)),
        source: None,
        limit: 20,
    };
    let on_tuesday = search(neomem, proof, tuesday)?;
    proof.claim(
        "\"what did I say on Tuesday\": a date window with no keywords lists exactly that day's records",
        on_tuesday.len() == 2,
        format!("{:?}", on_tuesday.iter().map(|hit| &hit.content).collect::<Vec<_>>()),
    );
    let mut from_codex = query("signing");
    from_codex.source = Some(Source::new("codex")?);
    let codex_hits = search(neomem, proof, from_codex)?;
    proof.claim(
        "a source filter keeps only that source's records",
        codex_hits.len() == 1 && codex_hits.iter().all(|hit| hit.source.as_str() == "codex"),
        codex_hits.len(),
    );
    Ok(())
}

fn old_records_rank_like_new_ones(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let hits = search(neomem, proof, query("expired certificate"))?;
    proof.claim(
        "a six-year-old record that matches best comes first (raw search has no decay)",
        hits.first().is_some_and(|hit| hit.content.starts_with("Back in 2020")),
        format!("{:?}", hits.iter().map(|hit| &hit.content).collect::<Vec<_>>()),
    );
    Ok(())
}

fn the_command_line_prints_the_hits(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let output = neomem.neomem(&["raw", "paywall", "--from", "2026-10-20", "--to", "2026-10-21"])?;
    let printed = String::from_utf8_lossy(&output.stdout);
    proof.claim(
        "neomem raw prints matching records with their source and time",
        output.status.success()
            && printed.contains("Renamed the paywall component")
            && printed.contains("claude-code"),
        printed.trim(),
    );
    Ok(())
}

fn raw_search_stays_fast_over_thousands_of_records(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let mut client = neomem.client()?;
    for index in 0..5_000_i64 {
        let content =
            format!("routine note {index} about builds, deploys and reviews of project {}", index % 37);
        client.call(Request::Add(record("bulk", TUESDAY - index * 60_000, &content)?))?;
    }
    let latency = timing::measure(50, || {
        client.call(Request::Raw(query("deploys project 12")))?;
        Ok(())
    })?;
    proof.claim("raw search over 5,000+ records answers under 30 ms at p95", latency.p95_ms < 30.0, latency);
    Ok(())
}

fn search_falls_back_to_keywords_when_the_daemon_cannot_start(
    proof: &mut ProofRun,
    neomem: &mut IsolatedNeomem,
    binaries: &Binaries,
) -> anyhow::Result<()> {
    neomem.stop_daemon()?;
    let alone = tempfile::tempdir()?;
    let lone_client = alone.path().join("neomem");
    std::fs::copy(&binaries.neomem, &lone_client)?;
    let output = std::process::Command::new(&lone_client)
        .args(["search", "signing error certificate"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", alone.path())
        .env("NEOMEM_HOME", neomem.home().root())
        .output()?;
    let printed = String::from_utf8_lossy(&output.stdout);
    let warned = String::from_utf8_lossy(&output.stderr);
    proof.claim(
        "with the daemon down and unable to start, neomem search falls back to keyword search over the intake log with a warning",
        output.status.success()
            && printed.contains("Fixed the signing error by rotating the Edge distribution certificate")
            && warned.contains("neomemd is not running"),
        format!("{} | {}", printed.lines().next().unwrap_or_default(), warned.trim()),
    );
    Ok(())
}
