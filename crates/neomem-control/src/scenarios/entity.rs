use std::fs;

use neomem_core::{EpochMs, IntakeId, IntakeRecord, MemoryId, Source, Span};
use neomem_proto::{EntityView, Request, Response, ShownMemory};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;
use crate::seed;

const EDGE: &str = "ent_01M56PGSJG83SYHRJXJG7DJ13N";
const EDGE_IOS: &str = "ent_01M56PGSJG83SYHRJXJG7DJ14P";
const OCTOBER: &str = "fct_01M56PJAD08JBX4FR84G98PBSK";
const NOVEMBER: &str = "fct_01M59NY2N09AC5V7E9Z0C1HT0H";
const SOURCED: &str = "fct_01M59NY2N09AC5V7E9Z0C1HT0K";

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = seed::start_with_vault_fixture("edge-ship-date", binaries)?;
    let home = neomem.home().root().to_path_buf();
    let source = log_source(&neomem)?;
    write_merged_duplicate_and_sourced_fact(&home, source)?;
    support::wait_for_index(&neomem, proof, 3, 3)?;
    names_resolve_to_entities(proof, &neomem)?;
    merged_names_and_ids_resolve_to_the_survivor(proof, &neomem)?;
    the_entity_view_shows_current_facts_and_links(proof, &neomem)?;
    show_returns_the_file_and_its_sources(proof, &neomem, source)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn log_source(neomem: &IsolatedNeomem) -> anyhow::Result<IntakeId> {
    let record = IntakeRecord {
        source: Source::new("note")?,
        occurred: Span::instant(EpochMs::from_millis(1_792_400_000_000)),
        tags: Vec::new(),
        content: "Johan: the Edge launch event moved to November 2".to_owned(),
        entity_hints: vec!["Edge".to_owned()],
        source_ref: None,
        session: None,
    };
    match neomem.client()?.call(Request::Add(record))? {
        Response::Added(receipt) => Ok(receipt.id),
        other => anyhow::bail!("add answered {other:?}"),
    }
}

fn write_merged_duplicate_and_sourced_fact(home: &std::path::Path, source: IntakeId) -> anyhow::Result<()> {
    let duplicate = format!(
        "---\nid: {EDGE_IOS}\nkind: entity\nentity_kind: project\nname: Edge iOS\naliases: []\nrecorded_at: 1792300051000\nupdated_at: 1792300052000\nmerged_from: []\nmerged_into: {EDGE}\nrelations: []\nlinks: []\n---\n"
    );
    fs::write(home.join("entities/project/edge-ios.md"), duplicate)?;
    let sourced = format!(
        "---\nid: {SOURCED}\nkind: fact\ntags: [code]\nabout: [{EDGE}]\nrecorded_at: 1792400100000\nupdated_at: 1792400100000\nvalid_at: 1792400000000\ninvalid_at: null\nimportance: 6\ndurability: ordinary\nsources: [{source}]\nlinks: []\nevidence: []\n---\nThe Edge launch event is on November 2, 2026.\n"
    );
    fs::write(home.join(format!("facts/2026/10/{SOURCED}_edge-launch-event.md")), sourced)?;
    Ok(())
}

fn resolve(neomem: &IsolatedNeomem, name: &str) -> anyhow::Result<(bool, String)> {
    let output = neomem.neomem(&["resolve", name, "--json"])?;
    Ok((output.status.success(), String::from_utf8_lossy(&output.stdout).trim().to_owned()))
}

fn names_resolve_to_entities(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let (by_alias_ok, by_alias) = resolve(neomem, "edge APP")?;
    proof.claim(
        "an alias resolves to its entity, case-insensitively",
        by_alias_ok && by_alias.contains(EDGE),
        &by_alias,
    );
    let (unknown_ok, unknown) = resolve(neomem, "Nobody Known")?;
    proof.claim("an unknown name does not resolve", !unknown_ok, unknown);
    Ok(())
}

fn merged_names_and_ids_resolve_to_the_survivor(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let (by_old_name_ok, by_old_name) = resolve(neomem, "Edge iOS")?;
    let (by_old_id_ok, by_old_id) = resolve(neomem, EDGE_IOS)?;
    proof.claim(
        "a merged entity's old name and old ID both resolve to the survivor",
        by_old_name_ok
            && by_old_id_ok
            && by_old_name.contains(EDGE)
            && by_old_id.contains(EDGE)
            && !by_old_id.contains(EDGE_IOS),
        format!("{by_old_name} / {by_old_id}"),
    );
    Ok(())
}

fn entity_view(
    neomem: &IsolatedNeomem,
    proof: &mut ProofRun,
    name: &str,
) -> anyhow::Result<Option<EntityView>> {
    Ok(match neomem.call(proof, Request::Entity { name: name.to_owned() })? {
        Response::Entity(view) => Some(*view),
        _ => None,
    })
}

fn the_entity_view_shows_current_facts_and_links(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<()> {
    let view = entity_view(neomem, proof, "Edge")?;
    let fact_ids: Vec<String> =
        view.iter().flat_map(|view| view.facts.iter().map(|fact| fact.id().to_string())).collect();
    proof.claim(
        "neomem entity lists the entity's current facts and leaves the replaced version out",
        view.as_ref()
            .is_some_and(|view| view.entity.name() == "Edge" && view.entity.aliases() == ["Edge app"])
            && fact_ids.contains(&NOVEMBER.to_owned())
            && fact_ids.contains(&SOURCED.to_owned())
            && !fact_ids.contains(&OCTOBER.to_owned()),
        format!("{fact_ids:?}"),
    );
    let linked: Vec<String> = view
        .iter()
        .flat_map(|view| view.linked.iter().map(|link| format!("{} {}", link.kind, link.entity.name())))
        .collect();
    proof.claim(
        "it lists linked entities with the link's label",
        linked.iter().any(|link| link == "owns Johan"),
        format!("{linked:?}"),
    );
    let merged_view = entity_view(neomem, proof, EDGE_IOS)?;
    proof.claim(
        "asking for a merged entity's old ID shows the survivor",
        merged_view.as_ref().is_some_and(|view| view.entity.id().to_string() == EDGE),
        merged_view.map_or_else(|| "none".to_owned(), |view| view.entity.id().to_string()),
    );
    let printed = neomem.neomem(&["entity", "edge app"])?;
    let text = String::from_utf8_lossy(&printed.stdout);
    proof.claim(
        "the command line prints the entity with its facts",
        printed.status.success() && text.contains("Edge") && text.contains("November 1, 2026"),
        text.lines().take(6).collect::<Vec<_>>().join(" | "),
    );
    Ok(())
}

fn show(neomem: &IsolatedNeomem, proof: &mut ProofRun, id: &str) -> anyhow::Result<Option<ShownMemory>> {
    let id: MemoryId = id.parse()?;
    Ok(match neomem.call(proof, Request::Show { id })? {
        Response::Shown(shown) => Some(*shown),
        _ => None,
    })
}

fn show_returns_the_file_and_its_sources(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    source: IntakeId,
) -> anyhow::Result<()> {
    let shown = show(neomem, proof, SOURCED)?;
    let on_disk = fs::read_to_string(
        neomem.home().root().join(format!("facts/2026/10/{SOURCED}_edge-launch-event.md")),
    )?;
    proof.claim(
        "neomem show returns the memory's file exactly as it is on disk",
        shown.as_ref().is_some_and(|shown| shown.text == on_disk),
        shown.as_ref().map_or_else(String::new, |shown| shown.path.clone()),
    );
    let sources_found = shown.as_ref().is_some_and(|shown| {
        shown.sources.len() == 1
            && shown.sources[0].id == source
            && shown.sources[0]
                .record
                .as_ref()
                .is_some_and(|record| record.content.contains("launch event moved"))
    });
    proof.claim(
        "it lists the fact's sources with the logged intake records",
        sources_found,
        format!("{:?}", shown.map(|shown| shown.sources.len())),
    );
    let entity = show(neomem, proof, EDGE)?;
    proof.claim(
        "an entity can be shown too",
        entity.is_some_and(|shown| shown.text.contains("name: Edge")),
        "",
    );
    Ok(())
}
