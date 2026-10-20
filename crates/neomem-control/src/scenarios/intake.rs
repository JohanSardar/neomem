use std::thread;

use neomem_core::{IntakeId, LoggedIntake};
use neomem_proto::Response;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::timing;

const SLEEP_REPORT: &str = "Slept 7h 12m, readiness 84. Woke once at 03:10.";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    let first = a_record_is_logged_exactly_as_sent(proof, &neomem)?;
    an_extraction_job_is_queued(proof, &neomem, first)?;
    the_same_record_twice_is_logged_twice(proof, &neomem)?;
    invalid_records_are_refused_and_not_logged(proof, &neomem)?;
    secrets_never_reach_the_log(proof, &neomem)?;
    parallel_adds_all_land_with_distinct_ids(proof, &neomem);
    adds_answer_inside_the_capture_budget(proof, &neomem)?;
    records_survive_a_restart(proof, &mut neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn add(neomem: &IsolatedNeomem, arguments: &[&str]) -> anyhow::Result<Option<IntakeId>> {
    let mut full = vec!["add"];
    full.extend_from_slice(arguments);
    full.push("--json");
    let output = neomem.neomem(&full)?;
    let response: Option<Response> = serde_json::from_slice(&output.stdout).ok();
    Ok(match response {
        Some(Response::Added(receipt)) if output.status.success() => Some(receipt.id),
        _ => None,
    })
}

fn display(id: Option<IntakeId>) -> String {
    id.map_or_else(|| "none".to_owned(), |id| id.to_string())
}

fn sleep_report_arguments() -> Vec<&'static str> {
    vec![
        SLEEP_REPORT,
        "--source",
        "oura-digest",
        "--tags",
        "health,sleep",
        "--start",
        "2026-10-19T22:40:00Z",
        "--end",
        "2026-10-20T06:52:00Z",
        "--entity",
        "Johan",
        "--entity",
        "Oura",
        "--ref",
        "oura://sleep/2026-10-20",
    ]
}

fn state_db(neomem: &IsolatedNeomem) -> anyhow::Result<Connection> {
    Ok(Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

fn logged(neomem: &IsolatedNeomem, id: IntakeId) -> anyhow::Result<Option<LoggedIntake>> {
    let connection = state_db(neomem)?;
    let row: Option<String> = connection
        .query_row(
            "SELECT json_object('id', id, 'recorded_at', recorded_at, 'record', json_object(
                'source', source, 'occurred', json_object('start', occurred_start, 'end', occurred_end),
                'tags', json(tags), 'content', content, 'entity_hints', json(entity_hints),
                'source_ref', source_ref, 'session', session)) FROM intake WHERE id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(row.map(|json| serde_json::from_str(&json)).transpose()?)
}

fn intake_count(neomem: &IsolatedNeomem) -> anyhow::Result<i64> {
    Ok(state_db(neomem)?.query_row("SELECT count(*) FROM intake", [], |row| row.get(0))?)
}

fn a_record_is_logged_exactly_as_sent(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<IntakeId> {
    let id = add(neomem, &sleep_report_arguments())?;
    proof.claim("neomem add returns an int_ ID", id.is_some(), display(id));
    let id = id.ok_or_else(|| anyhow::anyhow!("add returned no ID"))?;
    let record = logged(neomem, id)?.map(|logged| logged.record);
    let exact = record.as_ref().is_some_and(|record| {
        record.content == SLEEP_REPORT
            && record.source.as_str() == "oura-digest"
            && record.tags.iter().map(ToString::to_string).collect::<Vec<_>>() == ["health", "sleep"]
            && record.occurred.start.as_millis() == 1_792_449_600_000
            && record.occurred.end.as_millis() == 1_792_479_120_000
            && record.entity_hints == ["Johan", "Oura"]
            && record.source_ref.as_deref() == Some("oura://sleep/2026-10-20")
    });
    proof.claim("the intake log holds the record exactly as it was sent", exact, format!("{record:?}"));
    Ok(id)
}

fn an_extraction_job_is_queued(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    id: IntakeId,
) -> anyhow::Result<()> {
    let state: Option<String> = state_db(neomem)?
        .query_row(
            "SELECT state FROM jobs WHERE kind = 'extract' AND intake_id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    proof.claim(
        "an extraction job is queued for the record",
        state.as_deref() == Some("pending"),
        format!("{state:?}"),
    );
    Ok(())
}

fn the_same_record_twice_is_logged_twice(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let before = intake_count(neomem)?;
    let first = add(neomem, &sleep_report_arguments())?;
    let second = add(neomem, &sleep_report_arguments())?;
    let after = intake_count(neomem)?;
    proof.claim(
        "the same record sent twice is logged twice with two IDs (no dedupe keys)",
        first.is_some() && second.is_some() && first != second && after == before + 2,
        format!("{} {}, {before} -> {after}", display(first), display(second)),
    );
    Ok(())
}

fn invalid_records_are_refused_and_not_logged(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let before = intake_count(neomem)?;
    let empty = add(neomem, &["   ", "--source", "note"])?;
    let backwards =
        add(neomem, &["text", "--source", "note", "--start", "2026-10-20", "--end", "2026-10-19"])?;
    let bad_source = add(neomem, &["text", "--source", "Bad Source"])?;
    let after = intake_count(neomem)?;
    proof.claim(
        "empty content, a backwards span and a malformed source are refused and nothing is logged",
        empty.is_none() && backwards.is_none() && bad_source.is_none() && after == before,
        format!("{before} -> {after}"),
    );
    Ok(())
}

fn secrets_never_reach_the_log(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let secret = ["sk-proj-", "Zq81xYvB2nT7kLm3Pq9RsUv4WxYz0AbCdEfGh"].concat();
    let content = format!("Set OPENAI_API_KEY={secret} in the deploy env");
    let id = add(neomem, &[&content, "--source", "note"])?.ok_or_else(|| anyhow::anyhow!("add failed"))?;
    let record = logged(neomem, id)?.map(|logged| logged.record.content).unwrap_or_default();
    proof.claim("an API key in the content is removed before it is logged", !record.contains(&secret), record);
    Ok(())
}

fn parallel_adds_all_land_with_distinct_ids(proof: &mut ProofRun, neomem: &IsolatedNeomem) {
    let ids: Vec<Option<IntakeId>> = thread::scope(|scope| {
        let workers: Vec<_> = (0..40)
            .map(|index| {
                scope.spawn(move || {
                    let content = format!("parallel note {index}");
                    add(neomem, &[&content, "--source", "note"]).ok().flatten()
                })
            })
            .collect();
        workers.into_iter().map(|worker| worker.join().ok().flatten()).collect()
    });
    let distinct: std::collections::BTreeSet<_> = ids.iter().flatten().collect();
    proof.claim(
        "40 parallel adds all land with 40 distinct IDs",
        distinct.len() == 40,
        format!("{} distinct", distinct.len()),
    );
}

fn adds_answer_inside_the_capture_budget(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let latency = timing::measure(20, || {
        add(neomem, &["budget probe", "--source", "note"])?;
        Ok(())
    })?;
    proof.claim(
        "neomem add returns well inside the 50 ms capture budget at p95",
        latency.p95_ms < 50.0,
        latency,
    );
    Ok(())
}

fn records_survive_a_restart(proof: &mut ProofRun, neomem: &mut IsolatedNeomem) -> anyhow::Result<()> {
    let before = intake_count(neomem)?;
    neomem.stop_daemon()?;
    neomem.start_daemon()?;
    let after = intake_count(neomem)?;
    proof.claim(
        "every logged record is still there after a restart",
        before == after && before > 0,
        format!("{before} -> {after}"),
    );
    Ok(())
}
