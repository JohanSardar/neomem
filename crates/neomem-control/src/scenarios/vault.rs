use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use neomem_proto::{Request, Response};
use rusqlite::{Connection, OpenFlags};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::seed;

const COMMIT_WAIT: Duration = Duration::from_secs(8);
const NOVEMBER_FACT: &str =
    "facts/2026/10/fct_01M59NY2N09AC5V7E9Z0C1HT0H_edge-s-ship-date-is-november-1-2026.md";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let home = neomem.home().root().to_path_buf();
    the_home_is_a_local_repository(proof, &home)?;
    seeded_files_are_committed_together(proof, &home)?;
    only_memory_files_are_tracked(proof, &home)?;
    hand_edits_are_noticed_and_committed(proof, &home)?;
    malformed_files_are_recorded_not_fatal(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn git(home: &Path, arguments: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").args(arguments).current_dir(home).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn commit_count(home: &Path) -> usize {
    git(home, &["rev-list", "--count", "HEAD"]).ok().and_then(|count| count.parse().ok()).unwrap_or(0)
}

fn wait_for_commits(home: &Path, at_least: usize) -> usize {
    let deadline = Instant::now() + COMMIT_WAIT;
    while commit_count(home) < at_least && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    thread::sleep(Duration::from_millis(1500));
    commit_count(home)
}

fn the_home_is_a_local_repository(proof: &mut ProofRun, home: &Path) -> anyhow::Result<()> {
    let remotes = git(home, &["remote"])?;
    proof.claim(
        "the home is a local git repository with no remote",
        home.join(".git").is_dir() && remotes.is_empty(),
        format!("remotes: [{remotes}]"),
    );
    let ignored = fs::read_to_string(home.join(".gitignore")).unwrap_or_default();
    let required = ["state.db", "index.db", "models/", "run/", "spool/", "secrets.env", "*.env"];
    let missing: Vec<&str> =
        required.iter().copied().filter(|entry| !ignored.lines().any(|line| line == *entry)).collect();
    proof.claim(
        ".gitignore keeps databases, models, sockets, spool and secrets out of git",
        missing.is_empty(),
        format!("missing: {missing:?}"),
    );
    proof.claim(
        "facts/ and entities/ exist",
        home.join("facts").is_dir() && home.join("entities").is_dir(),
        home.display(),
    );
    Ok(())
}

fn seeded_files_are_committed_together(proof: &mut ProofRun, home: &Path) -> anyhow::Result<()> {
    let before = commit_count(home);
    let copied = seed::copy_vault_fixture("edge-ship-date", home)?;
    let after = wait_for_commits(home, before + 1);
    let tracked = git(home, &["ls-files"])?;
    let all_tracked = copied.iter().all(|file| tracked.lines().any(|line| Path::new(line) == file));
    proof.claim(
        "a seeded fact chain is committed by the daemon within the batch interval, in one commit",
        all_tracked && after == before + 1,
        format!("{} files, commits {before} -> {after}", copied.len()),
    );
    Ok(())
}

fn only_memory_files_are_tracked(proof: &mut ProofRun, home: &Path) -> anyhow::Result<()> {
    let tracked = git(home, &["ls-files"])?;
    let strays: Vec<&str> = tracked
        .lines()
        .filter(|line| {
            !(line.starts_with("facts/")
                || line.starts_with("entities/")
                || *line == "config.toml"
                || *line == ".gitignore")
        })
        .collect();
    proof.claim(
        "only facts, entities, config.toml and .gitignore are ever committed",
        strays.is_empty(),
        format!("strays: {strays:?}"),
    );
    Ok(())
}

fn hand_edits_are_noticed_and_committed(proof: &mut ProofRun, home: &Path) -> anyhow::Result<()> {
    let before = commit_count(home);
    let path = home.join(NOVEMBER_FACT);
    let edited = fs::read_to_string(&path)?.replace("importance: 7", "importance: 8");
    fs::write(&path, edited)?;
    let after = wait_for_commits(home, before + 1);
    let last = git(home, &["log", "-1", "--name-only", "--format="])?;
    proof.claim(
        "a hand-edited fact file is noticed and committed",
        after == before + 1 && last.lines().any(|line| line == NOVEMBER_FACT),
        format!("commits {before} -> {after}, last commit touched [{last}]"),
    );
    Ok(())
}

fn malformed_files_are_recorded_not_fatal(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let home = neomem.home().root();
    let broken = home.join("facts/2026/10/fct_01M5AAAAAAAAAAAAAAAAAAAAAA_broken.md");
    fs::write(&broken, "---\nid: fct_01M5AAAAAAAAAAAAAAAAAAAAAA\nkind: fact\n---\nno metadata\n")?;
    thread::sleep(Duration::from_millis(1500));
    let still_serving = matches!(neomem.call(proof, Request::Status)?, Response::Status(_));
    let state = Connection::open_with_flags(neomem.home().state_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let recorded: Vec<String> = state
        .prepare("SELECT subject FROM errors WHERE kind = 'vault_unreadable'")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let content_free = recorded.iter().all(|subject| !subject.contains("no metadata"));
    proof.claim(
        "a malformed file neither stops the daemon nor gets trusted, and its error log names only the file",
        still_serving
            && recorded.iter().any(|subject| subject.contains("fct_01M5AAAAAAAAAAAAAAAAAAAAAA"))
            && content_free,
        format!("errors: {recorded:?}"),
    );
    Ok(())
}
