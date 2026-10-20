use neomem_core::{EpochMs, IntakeId, IntakeRecord, Source, Span};
use neomem_proto::{RawQuery, Request, Response};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;

const WRONG_NIGHT: &str = "Slept 4h 05m, readiness 41 (this was the wrong night)";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let wrong = log_wrong_night(&neomem)?;
    let output = neomem.neomem(&[
        "correct",
        &wrong.to_string(),
        "--reason",
        "the automation sent the wrong night",
        "--json",
    ])?;
    let reply: Option<Response> = serde_json::from_slice(&output.stdout).ok();
    proof.claim(
        "neomem correct records a correction for the intake record",
        output.status.success()
            && matches!(reply, Some(Response::Corrected(receipt)) if receipt.intake == wrong),
        String::from_utf8_lossy(&output.stdout).trim(),
    );
    the_record_itself_is_never_edited(proof, &neomem, wrong)?;
    raw_search_flags_it(proof, &neomem)?;
    its_extraction_is_skipped_and_a_recheck_is_queued(proof, &neomem, wrong)?;
    unknown_records_cannot_be_corrected(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn log_wrong_night(neomem: &IsolatedNeomem) -> anyhow::Result<IntakeId> {
    let record = IntakeRecord {
        source: Source::new("oura-digest")?,
        occurred: Span::instant(EpochMs::from_millis(1_792_540_800_000)),
        tags: Vec::new(),
        content: WRONG_NIGHT.to_owned(),
        entity_hints: Vec::new(),
        source_ref: None,
        session: None,
    };
    match neomem.client()?.call(Request::Add(record))? {
        Response::Added(receipt) => Ok(receipt.id),
        other => anyhow::bail!("add answered {other:?}"),
    }
}

fn state(neomem: &IsolatedNeomem) -> anyhow::Result<Connection> {
    Ok(Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?)
}

fn the_record_itself_is_never_edited(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    wrong: IntakeId,
) -> anyhow::Result<()> {
    let content: Option<String> = state(neomem)?
        .query_row("SELECT content FROM intake WHERE id = ?1", [wrong.to_string()], |row| row.get(0))
        .optional()?;
    proof.claim(
        "the intake record is kept exactly as it was logged",
        content.as_deref() == Some(WRONG_NIGHT),
        format!("{content:?}"),
    );
    Ok(())
}

fn raw_search_flags_it(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let query = RawQuery { text: "readiness".to_owned(), from: None, to: None, source: None, limit: 20 };
    let flagged = match neomem.call(proof, Request::Raw(query))? {
        Response::RawHits(hits) => hits.iter().any(|hit| hit.content == WRONG_NIGHT && hit.corrected),
        _ => false,
    };
    proof.claim("raw search still finds the record and marks it corrected", flagged, flagged);
    Ok(())
}

fn its_extraction_is_skipped_and_a_recheck_is_queued(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    wrong: IntakeId,
) -> anyhow::Result<()> {
    let connection = state(neomem)?;
    let job_state = |kind: &str| -> anyhow::Result<Option<String>> {
        Ok(connection
            .query_row(
                "SELECT state FROM jobs WHERE kind = ?1 AND intake_id = ?2",
                [kind, &wrong.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    };
    let extraction = job_state("extract")?;
    let recheck = job_state("recheck")?;
    proof.claim(
        "its pending extraction is skipped and a re-check of facts drawn from it is queued",
        extraction.as_deref() == Some("skipped") && recheck.as_deref() == Some("pending"),
        format!("extract {extraction:?}, recheck {recheck:?}"),
    );
    Ok(())
}

fn unknown_records_cannot_be_corrected(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let ghost = IntakeId::from_parts(EpochMs::from_millis(1), 1);
    let output = neomem.neomem(&["correct", &ghost.to_string(), "--reason", "typo"])?;
    proof.claim(
        "correcting an unknown intake ID fails and records nothing",
        !output.status.success(),
        String::from_utf8_lossy(&output.stderr).trim(),
    );
    Ok(())
}
