use std::fs;
use std::path::Path;

use neomem_core::{EntityId, EpochMs, FactId, IntakeId, IntakeRecord, Source, Span};
use neomem_proto::{Request, Response, SearchHit, SearchQuery};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;

const DAY: i64 = 86_400_000;
const START: i64 = 1_780_000_000_000;
const LATE_CHAIN: usize = 6;

struct Chain {
    length: usize,
    versions: Vec<FactId>,
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let home = neomem.home().root().to_path_buf();
    let chains = write_chains_newest_first(&home)?;
    let late = write_a_late_version(&home, &chains[LATE_CHAIN - 2])?;
    let facts: usize = chains.iter().map(|chain| chain.length).sum::<usize>() + 1;
    support::wait_for_index(&neomem, proof, facts, chains.len())?;
    every_chain_answers_with_its_newest_version(proof, &neomem, &chains)?;
    the_late_version_never_displaces_the_newest(proof, &neomem, &chains[LATE_CHAIN - 2], late)?;
    as_of_answers_with_the_version_valid_then(proof, &neomem, &chains[3])?;
    include_invalid_shows_the_old_versions(proof, &neomem, &chains[3])?;
    corrected_facts_are_never_returned(proof, &neomem, &home)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn wording(length: usize, version: usize) -> String {
    format!("The release codename of project Kestrel{length} is Falcon{length}v{version}.")
}

fn days(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(0) * DAY
}

fn entity_id(length: usize) -> EntityId {
    EntityId::from_parts(EpochMs::from_millis(START), 9000 + u128::try_from(length).unwrap_or(0))
}

fn write_chains_newest_first(home: &Path) -> anyhow::Result<Vec<Chain>> {
    let mut chains = Vec::new();
    for length in 2..=10 {
        let entity = entity_id(length);
        write_entity(home, entity, &format!("Kestrel{length}"))?;
        let versions: Vec<FactId> = (1..=length)
            .map(|version| {
                FactId::from_parts(
                    EpochMs::from_millis(START + days(version)),
                    u128::try_from(length * 100 + version).unwrap_or(0),
                )
            })
            .collect();
        for version in (1..=length).rev() {
            let valid_at = START + days(version);
            let invalid_at = (version < length).then(|| valid_at + DAY);
            let updates = (version > 1).then(|| versions[version - 2]);
            write_fact(
                home,
                versions[version - 1],
                entity,
                &wording(length, version),
                valid_at,
                invalid_at,
                updates,
            )?;
        }
        chains.push(Chain { length, versions });
    }
    Ok(chains)
}

fn write_a_late_version(home: &Path, chain: &Chain) -> anyhow::Result<FactId> {
    let late = FactId::from_parts(EpochMs::from_millis(START + 2 * DAY + 3_600_000), 77_777);
    let content = format!(
        "The release codename of project Kestrel{} was briefly Falcon{}late.",
        chain.length, chain.length
    );
    write_fact(
        home,
        late,
        entity_id(chain.length),
        &content,
        START + 2 * DAY + 3_600_000,
        None,
        Some(chain.versions[0]),
    )?;
    Ok(late)
}

fn write_entity(home: &Path, id: EntityId, name: &str) -> anyhow::Result<()> {
    let directory = home.join("entities/project");
    fs::create_dir_all(&directory)?;
    let text = format!(
        "---\nid: {id}\nkind: entity\nentity_kind: project\nname: {name}\naliases: []\nrecorded_at: {START}\nupdated_at: {START}\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
    );
    fs::write(directory.join(format!("{}.md", name.to_lowercase())), text)?;
    Ok(())
}

fn write_fact(
    home: &Path,
    id: FactId,
    about: EntityId,
    content: &str,
    valid_at: i64,
    invalid_at: Option<i64>,
    updates: Option<FactId>,
) -> anyhow::Result<()> {
    let directory = home.join("facts/2026/06");
    fs::create_dir_all(&directory)?;
    let source = IntakeId::from_parts(EpochMs::from_millis(valid_at), 1);
    let invalid = invalid_at.map_or_else(|| "null".to_owned(), |at| at.to_string());
    let links = updates.map_or_else(|| "[]".to_owned(), |old| format!("[{{type: updates, to: {old}}}]"));
    let text = format!(
        "---\nid: {id}\nkind: fact\ntags: [code]\nabout: [{about}]\nrecorded_at: {valid_at}\nupdated_at: {valid_at}\nvalid_at: {valid_at}\ninvalid_at: {invalid}\nimportance: 5\ndurability: ordinary\nsources: [{source}]\nlinks: {links}\nevidence: []\n---\n{content}\n"
    );
    fs::write(directory.join(format!("{id}_fact.md")), text)?;
    Ok(())
}

fn search(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    query: SearchQuery,
) -> anyhow::Result<Vec<SearchHit>> {
    Ok(match neomem.call(proof, Request::Search(query))? {
        Response::SearchResults(results) => results.hits,
        _ => Vec::new(),
    })
}

fn asking(text: &str) -> SearchQuery {
    SearchQuery { text: text.to_owned(), ..SearchQuery::default() }
}

fn every_chain_answers_with_its_newest_version(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    chains: &[Chain],
) -> anyhow::Result<()> {
    let mut first_is_newest = 0;
    let mut no_old_versions = 0;
    for chain in chains {
        let hits = search(neomem, proof, asking(&wording(chain.length, 1)))?;
        let newest = chain.versions[chain.length - 1];
        if hits.first().is_some_and(|hit| hit.fact.id() == newest) {
            first_is_newest += 1;
        }
        if !hits.iter().any(|hit| chain.versions[..chain.length - 1].contains(&hit.fact.id())) {
            no_old_versions += 1;
        }
    }
    proof.claim(
        "for chains of 2 to 10 updates written out of order, a query worded like the oldest version returns the newest first",
        first_is_newest == chains.len(),
        format!("{first_is_newest}/{}", chains.len()),
    );
    proof.claim(
        "no replaced version ever appears in the results",
        no_old_versions == chains.len(),
        format!("{no_old_versions}/{}", chains.len()),
    );
    Ok(())
}

fn the_late_version_never_displaces_the_newest(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    chain: &Chain,
    late: FactId,
) -> anyhow::Result<()> {
    let hits = search(
        neomem,
        proof,
        asking(&format!(
            "The release codename of project Kestrel{} was briefly Falcon{}late.",
            chain.length, chain.length
        )),
    )?;
    let newest = chain.versions[chain.length - 1];
    proof.claim(
        "a late version valid in the middle of the chain, arriving last, answers with the chain's newest version instead",
        hits.first().is_some_and(|hit| hit.fact.id() == newest) && !hits.iter().any(|hit| hit.fact.id() == late),
        hits.first().map_or_else(String::new, |hit| hit.fact.content().to_owned()),
    );
    Ok(())
}

fn as_of_answers_with_the_version_valid_then(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    chain: &Chain,
) -> anyhow::Result<()> {
    let moment = EpochMs::from_millis(START + 2 * DAY + DAY / 2);
    let hits =
        search(neomem, proof, SearchQuery { as_of: Some(moment), ..asking(&wording(chain.length, 1)) })?;
    proof.claim(
        "--as-of a moment between versions 2 and 3 returns version 2, the one valid then",
        hits.first().is_some_and(|hit| hit.fact.id() == chain.versions[1]),
        hits.first().map_or_else(String::new, |hit| hit.fact.content().to_owned()),
    );
    Ok(())
}

fn include_invalid_shows_the_old_versions(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    chain: &Chain,
) -> anyhow::Result<()> {
    let hits =
        search(neomem, proof, SearchQuery { include_invalid: true, ..asking(&wording(chain.length, 1)) })?;
    proof.claim(
        "--include-invalid also returns the old version a query matches (ranked at a fifth, as §8 scores invalid facts)",
        hits.iter().any(|hit| hit.fact.id() == chain.versions[0] && hit.fact.invalid_at().is_some()),
        "",
    );
    Ok(())
}

fn corrected_facts_are_never_returned(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    home: &Path,
) -> anyhow::Result<()> {
    let record = IntakeRecord {
        source: Source::new("note")?,
        occurred: Span::instant(EpochMs::from_millis(START)),
        tags: Vec::new(),
        content: "The Osprey project budget is 40 thousand".to_owned(),
        entity_hints: Vec::new(),
        source_ref: None,
        session: None,
    };
    let Response::Added(receipt) = neomem.client()?.call(Request::Add(record))? else {
        anyhow::bail!("add failed")
    };
    let entity = entity_id(11);
    write_entity(home, entity, "Osprey")?;
    let fact = FactId::from_parts(EpochMs::from_millis(START + 20 * DAY), 88_888);
    let content = "The Osprey project budget is 40 thousand euros.";
    let text = format!(
        "---\nid: {fact}\nkind: fact\ntags: [money]\nabout: [{entity}]\nrecorded_at: {START}\nupdated_at: {START}\nvalid_at: {START}\ninvalid_at: null\nimportance: 5\ndurability: ordinary\nsources: [{}]\nlinks: []\nevidence: []\n---\n{content}\n",
        receipt.id
    );
    fs::write(home.join(format!("facts/2026/06/{fact}_osprey.md")), text)?;
    support::wait_for_index(neomem, proof, 57, 11)?;
    let before = search(neomem, proof, asking(content))?.iter().any(|hit| hit.fact.id() == fact);
    neomem.client()?.call(Request::Correct { intake: receipt.id, reason: "wrong project".to_owned() })?;
    let after = search(neomem, proof, asking(content))?.iter().any(|hit| hit.fact.id() == fact);
    proof.claim(
        "a fact whose only source was corrected disappears from search",
        before && !after,
        format!("before {before}, after {after}"),
    );
    Ok(())
}
