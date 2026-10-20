use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use neomem_proto::Response;
use rusqlite::{Connection, OpenFlags};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;

const DRAIN_WAIT: Duration = Duration::from_secs(5);

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let mut neomem = IsolatedNeomem::start(binaries)?;
    neomem.stop_daemon()?;
    adds_while_down_are_spooled_at_once(proof, &neomem)?;
    a_corrupt_spool_file_is_set_aside(proof, &neomem)?;
    neomem.start_daemon()?;
    the_spool_drains_into_the_intake_log(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn spool_files(neomem: &IsolatedNeomem) -> Vec<String> {
    fs::read_dir(neomem.home().spool_dir())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_file())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn adds_while_down_are_spooled_at_once(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let started = Instant::now();
    let first = neomem.neomem(&["add", "Spooled while the daemon was down", "--source", "note", "--json"])?;
    let elapsed = started.elapsed();
    let second =
        neomem.neomem(&["add", "Spooled while the daemon was down", "--source", "note", "--json"])?;
    let files = spool_files(neomem);
    let all_json = files
        .iter()
        .all(|name| std::path::Path::new(name).extension().is_some_and(|extension| extension == "json"));
    proof.claim(
        "neomem add with the daemon down exits 0 at once and leaves one spool file per add",
        first.status.success()
            && second.status.success()
            && files.len() == 2
            && all_json
            && elapsed < Duration::from_millis(50),
        format!("{elapsed:?}, files {files:?}, stdout {}", String::from_utf8_lossy(&first.stdout).trim()),
    );
    let reply: Option<Response> = serde_json::from_slice(&first.stdout).ok();
    proof.claim(
        "a spooled add says so instead of inventing an ID",
        reply.is_none() && String::from_utf8_lossy(&first.stdout).contains("spooled"),
        String::from_utf8_lossy(&first.stdout),
    );
    Ok(())
}

fn a_corrupt_spool_file_is_set_aside(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    fs::write(neomem.home().spool_dir().join("0000-corrupt.json"), "{ not json")?;
    proof.claim(
        "a corrupt spool file is in place before the daemon starts",
        spool_files(neomem).len() == 3,
        "3 files",
    );
    Ok(())
}

fn the_spool_drains_into_the_intake_log(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let deadline = Instant::now() + DRAIN_WAIT;
    while !spool_files(neomem).is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let state = Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let logged: i64 = state.query_row(
        "SELECT count(*) FROM intake WHERE content = 'Spooled while the daemon was down'",
        [],
        |row| row.get(0),
    )?;
    let left = spool_files(neomem);
    let set_aside = neomem.home().spool_dir().join("rejected").join("0000-corrupt.json").exists();
    proof.claim(
        "after a restart both spooled adds are in the intake log, each once, and the spool is empty",
        logged == 2 && left.is_empty(),
        format!("logged {logged}, left {left:?}"),
    );
    proof.claim(
        "the corrupt file was moved to spool/rejected instead of blocking the drain",
        set_aside,
        set_aside,
    );
    Ok(())
}
