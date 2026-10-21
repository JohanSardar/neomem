use std::fs;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use neomem_core::EpochMs;
use neomem_proto::{IndexReport, Request, Response};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;

const NOVEMBER: &str = "fct_01M59NY2N09AC5V7E9Z0C1HT0H";
const JOHAN: &str = "ent_01M56PH3B01C00K6AGV0VFCXEC";
const A_DAY_MS: i64 = 86_400_000;
const RUN_WAIT: Duration = Duration::from_secs(10);

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = seed::start_with_vault_fixture("edge-ship-date", binaries)?;
    support::wait_for_index(&neomem, proof, 2, 2)?;
    neomem.stop_daemon()?;
    drift_behind_the_daemons_back(&neomem)?;
    neomem.start_daemon()?;
    let stale = support::index_report(&neomem, proof)?;
    proof.claim(
        "changes made while the daemon was down leave the index stale until the drift check",
        stale.as_ref().is_some_and(|report| report.facts == 2 && report.entities == 1),
        format!("{stale:?}"),
    );
    make_it_night(&neomem, proof)?;
    let run = wait_for_drift_runs(&neomem, 1)?;
    the_nightly_check_repairs_every_difference(proof, &neomem, run.as_deref())?;
    a_clean_index_has_no_drift(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn drift_behind_the_daemons_back(neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let facts = neomem.home().root().join("facts/2026/10");
    let november = facts.join(format!("{NOVEMBER}_edge-s-ship-date-is-november-1-2026.md"));
    let text = fs::read_to_string(&november)?;
    fs::write(&november, text.replace("November 1, 2026", "November 8, 2026"))?;
    let added = text
        .replace(NOVEMBER, "fct_01M59NY2N09AC5V7E9Z0C1HT0M")
        .replace("Edge's ship date is November 1, 2026.", "Edge's beta opens on October 25, 2026.")
        .replace("links: [{type: updates, to: fct_01M56PJAD08JBX4FR84G98PBSK}]", "links: []");
    fs::write(facts.join("fct_01M59NY2N09AC5V7E9Z0C1HT0M_edge-beta.md"), added)?;
    let index = Connection::open(neomem.home().index_db_path())?;
    index.execute("DELETE FROM memories WHERE id = ?1", [JOHAN])?;
    Ok(())
}

fn make_it_night(neomem: &IsolatedNeomem, proof: &mut ProofRun) -> anyhow::Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let tomorrow = EpochMs::from_millis(i64::try_from(now)? + A_DAY_MS);
    let moved = neomem.set_clock(tomorrow)?;
    proof.claim(
        "neomemctl set clock moves the isolated daemon's clock a day ahead",
        moved,
        tomorrow.as_millis(),
    );
    Ok(())
}

fn drift_runs(neomem: &IsolatedNeomem) -> anyhow::Result<Vec<String>> {
    let state = Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = state.prepare("SELECT changes FROM runs WHERE kind = 'drift' ORDER BY id")?;
    Ok(statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?)
}

fn wait_for_drift_runs(neomem: &IsolatedNeomem, count: usize) -> anyhow::Result<Option<String>> {
    let deadline = Instant::now() + RUN_WAIT;
    loop {
        let runs = drift_runs(neomem).unwrap_or_default();
        if runs.len() >= count || Instant::now() > deadline {
            return Ok(runs.get(count.saturating_sub(1)).cloned());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn the_nightly_check_repairs_every_difference(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    run: Option<&str>,
) -> anyhow::Result<()> {
    let report = support::index_report(neomem, proof)?;
    proof.claim(
        "the nightly drift check indexes the new file, the edited fact and the lost row",
        report.as_ref().is_some_and(|report: &IndexReport| {
            report.facts == 3 && report.entities == 2 && report.vectors == 5
        }),
        format!("{report:?}"),
    );
    let content: Option<String> =
        Connection::open_with_flags(neomem.home().index_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?
            .query_row("SELECT content FROM memories WHERE id = ?1", [NOVEMBER], |row| row.get(0))
            .optional()?;
    proof.claim(
        "index.db now holds the edited fact's new text",
        content.as_deref().is_some_and(|text| text.contains("November 8")),
        format!("{content:?}"),
    );
    let counted =
        run.is_some_and(|changes| changes.contains("\"differences\":3") && !changes.contains("Edge"));
    proof.claim(
        "the run log records the check with its counts and no memory content",
        counted,
        run.unwrap_or("no run"),
    );
    Ok(())
}

fn a_clean_index_has_no_drift(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let second = match neomem.client()?.call(Request::CheckDrift)? {
        Response::DriftChecked(report) => Some(report),
        _ => None,
    };
    proof.claim(
        "checking again finds no differences",
        second.as_ref().is_some_and(|report| report.differences == 0),
        format!("{second:?}"),
    );
    Ok(())
}
