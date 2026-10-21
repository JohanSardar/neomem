use std::fs;
use std::path::Path;

use neomem_core::{EntityId, EpochMs, FactId, IntakeId};
use neomem_proto::{HistoryView, Request, Response, SearchQuery};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;

const START: i64 = 1_786_000_000_000;
const DAY: i64 = 86_400_000;

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let (versions, extension) = write_chain(neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 4, 1)?;
    for (index, version) in versions.iter().enumerate() {
        let view = history(&neomem, proof, *version)?;
        let listed: Vec<FactId> =
            view.iter().flat_map(|view| view.versions.iter().map(neomem_core::Fact::id)).collect();
        if index == 0 {
            proof.claim(
                "neomem history lists every version of the chain, oldest first, with when each was valid",
                listed == versions
                    && view.as_ref().is_some_and(|view| {
                        view.versions.iter().take(2).all(|entry| entry.invalid_at().is_some())
                    }),
                format!("{} versions", listed.len()),
            );
            proof.claim(
                "it names the current version and the facts that extend the chain",
                view.as_ref().is_some_and(|view| {
                    view.current == versions[2] && view.extensions.iter().any(|fact| fact.id() == extension)
                }),
                "",
            );
        } else {
            proof.claim(
                &format!("asking from version {} gives the same history", index + 1),
                listed == versions,
                format!("{} versions", listed.len()),
            );
        }
    }
    let printed = neomem.neomem(&["history", &versions[0].to_string()])?;
    let text = String::from_utf8_lossy(&printed.stdout);
    proof.claim(
        "the command line prints the chain in order with its validity",
        printed.status.success()
            && text.find("Falcon v1").zip(text.find("Falcon v3")).is_some_and(|(first, last)| first < last)
            && text.contains("current"),
        text.lines().take(4).collect::<Vec<_>>().join(" | "),
    );
    let before = match neomem.call(
        proof,
        Request::Search(SearchQuery {
            text: "project Harrier codename".to_owned(),
            as_of: Some(EpochMs::from_millis(START - DAY)),
            ..SearchQuery::default()
        }),
    )? {
        Response::SearchResults(results) => results.hits.len(),
        _ => usize::MAX,
    };
    proof.claim("--as-of before the first version existed finds nothing from the chain", before == 0, before);
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn history(neomem: &IsolatedNeomem, proof: &mut ProofRun, id: FactId) -> anyhow::Result<Option<HistoryView>> {
    Ok(match neomem.call(proof, Request::History { id })? {
        Response::History(view) => Some(*view),
        _ => None,
    })
}

fn write_chain(home: &Path) -> anyhow::Result<(Vec<FactId>, FactId)> {
    let entity = EntityId::from_parts(EpochMs::from_millis(START), 5);
    fs::create_dir_all(home.join("entities/project"))?;
    fs::write(
        home.join("entities/project/harrier.md"),
        format!(
            "---\nid: {entity}\nkind: entity\nentity_kind: project\nname: Harrier\naliases: []\nrecorded_at: {START}\nupdated_at: {START}\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
        ),
    )?;
    let versions: Vec<FactId> = (1..=3_i64)
        .map(|number| {
            FactId::from_parts(
                EpochMs::from_millis(START + number * DAY),
                40 + u128::try_from(number).unwrap_or(0),
            )
        })
        .collect();
    let extension = FactId::from_parts(EpochMs::from_millis(START + 4 * DAY), 50);
    fs::create_dir_all(home.join("facts/2026/08"))?;
    for (index, id) in versions.iter().enumerate() {
        let number = i64::try_from(index).unwrap_or(0) + 1;
        let valid_at = START + number * DAY;
        let invalid = if number < 3 { (valid_at + DAY).to_string() } else { "null".to_owned() };
        let links = if index == 0 {
            "[]".to_owned()
        } else {
            format!("[{{type: updates, to: {}}}]", versions[index - 1])
        };
        write_fact(
            home,
            *id,
            entity,
            &format!("The codename of project Harrier is Falcon v{number}."),
            valid_at,
            &invalid,
            &links,
        )?;
    }
    write_fact(
        home,
        extension,
        entity,
        "The Falcon v3 codename was chosen by the design team.",
        START + 4 * DAY,
        "null",
        &format!("[{{type: extends, to: {}}}]", versions[2]),
    )?;
    Ok((versions, extension))
}

fn write_fact(
    home: &Path,
    id: FactId,
    entity: EntityId,
    content: &str,
    valid_at: i64,
    invalid: &str,
    links: &str,
) -> anyhow::Result<()> {
    let source = IntakeId::from_parts(EpochMs::from_millis(valid_at), 1);
    fs::write(
        home.join(format!("facts/2026/08/{id}_harrier.md")),
        format!(
            "---\nid: {id}\nkind: fact\ntags: [code]\nabout: [{entity}]\nrecorded_at: {valid_at}\nupdated_at: {valid_at}\nvalid_at: {valid_at}\ninvalid_at: {invalid}\nimportance: 5\ndurability: ordinary\nsources: [{source}]\nlinks: {links}\nevidence: []\n---\n{content}\n"
        ),
    )?;
    Ok(())
}
