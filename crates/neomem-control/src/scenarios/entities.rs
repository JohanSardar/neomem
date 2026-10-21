use std::fs;
use std::path::Path;

use neomem_core::{EntityId, EpochMs, FactId, IntakeId, LinkKind};
use neomem_proto::{DreamReport, EntityView, Request, Response};
use neomem_testkit::{FakeRules, LlmRule, RuleToolCall};
use serde_json::json;

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;

const KEEPER: &str = "You are neomem's entity keeper";
const START: i64 = 1_790_000_000_000;

struct People {
    sam: EntityId,
    samuel: EntityId,
    vega_fact: FactId,
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let people = seed(neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 2, 5)?;
    neomem.add_fake_rules(&FakeRules { llm: scripted_keeper(), ..FakeRules::default() })?;
    let report = match neomem.call(proof, Request::Dream)? {
        Response::Dreamed(report) => Some(report),
        _ => None,
    };
    merges_keep_every_name_and_resolve_old_ids(proof, &neomem, &people)?;
    aliases_and_links_land(proof, &neomem)?;
    recurring_names_become_entities_citing_facts(proof, &neomem, &people)?;
    the_keeper_is_limited(proof, report.as_ref());
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn entity(home: &Path, id: EntityId, kind: &str, name: &str) -> anyhow::Result<()> {
    fs::create_dir_all(home.join(format!("entities/{kind}")))?;
    fs::write(
        home.join(format!("entities/{kind}/{}.md", name.to_lowercase().replace(' ', "-"))),
        format!(
            "---\nid: {id}\nkind: entity\nentity_kind: {kind}\nname: {name}\naliases: []\nrecorded_at: {START}\nupdated_at: {START}\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
        ),
    )?;
    Ok(())
}

fn seed(home: &Path) -> anyhow::Result<People> {
    let id = |seed: u128| EntityId::from_parts(EpochMs::from_millis(START), seed);
    let (acme, edge, paywall, sam, samuel) = (id(1), id(2), id(3), id(4), id(5));
    entity(home, acme, "organization", "Acme Labs")?;
    entity(home, edge, "project", "Edge")?;
    entity(home, paywall, "thing", "Paywall")?;
    entity(home, sam, "person", "Sam Kim")?;
    entity(home, samuel, "person", "Samuel Kim")?;
    let vega_fact = FactId::from_parts(EpochMs::from_millis(START), 30);
    fs::create_dir_all(home.join("facts/2026/09"))?;
    for (fact, content, about) in [
        (vega_fact, "The Vega project launches in March with Sam Kim leading it.", sam),
        (
            FactId::from_parts(EpochMs::from_millis(START), 31),
            "Samuel Kim's email is sam@acme.example.",
            samuel,
        ),
    ] {
        let source = IntakeId::from_parts(EpochMs::from_millis(START), 1);
        fs::write(
            home.join(format!("facts/2026/09/{fact}_people.md")),
            format!(
                "---\nid: {fact}\nkind: fact\ntags: [social]\nabout: [{about}]\nrecorded_at: {START}\nupdated_at: {START}\nvalid_at: {START}\ninvalid_at: null\nimportance: 5\ndurability: ordinary\nsources: [{source}]\nlinks: []\nevidence: []\n---\n{content}\n"
            ),
        )?;
    }
    Ok(People { sam, samuel, vega_fact })
}

fn call(name: &str, arguments: serde_json::Value) -> RuleToolCall {
    RuleToolCall { name: name.to_owned(), arguments }
}

fn scripted_keeper() -> Vec<LlmRule> {
    let step = |step: usize, calls: Vec<RuleToolCall>| LlmRule {
        system_contains: Some(KEEPER.to_owned()),
        step: Some(step),
        times: Some(1),
        tool_calls: calls,
        ..LlmRule::default()
    };
    vec![
        step(
            0,
            vec![
                call("merge_entities", json!({ "keep": 4, "duplicate": 5 })),
                call("add_alias", json!({ "entity": 2, "alias": "Edge app" })),
                call("link_entity", json!({ "from": 3, "label": "part_of", "to": 2 })),
                call("link_entity", json!({ "from": 1, "label": "possibly_same", "to": 2 })),
                call("search_facts", json!({ "query": "Vega project launches" })),
            ],
        ),
        step(
            5,
            vec![
                call(
                    "create_entity",
                    json!({ "name": "Vega", "kind": "project", "aliases": [], "facts": [1] }),
                ),
                call(
                    "create_entity",
                    json!({ "name": "Orion", "kind": "project", "aliases": [], "facts": [] }),
                ),
                call("merge_entities", json!({ "keep": 4, "duplicate": 42 })),
                call("link", json!({ "from": 1, "kind": "updates", "to": 2 })),
            ],
        ),
        LlmRule {
            system_contains: Some(KEEPER.to_owned()),
            step: Some(9),
            times: Some(1),
            reply: "Merged Sam, tidied Edge, created Vega.".to_owned(),
            ..LlmRule::default()
        },
    ]
}

fn resolve(neomem: &IsolatedNeomem, proof: &mut ProofRun, name: &str) -> anyhow::Result<Vec<EntityId>> {
    Ok(match neomem.call(proof, Request::Resolve { name: name.to_owned() })? {
        Response::Resolved(found) => found.into_iter().map(|entity| entity.id).collect(),
        _ => Vec::new(),
    })
}

fn view(neomem: &IsolatedNeomem, proof: &mut ProofRun, name: &str) -> anyhow::Result<Option<EntityView>> {
    Ok(match neomem.call(proof, Request::Entity { name: name.to_owned() })? {
        Response::Entity(view) => Some(*view),
        _ => None,
    })
}

fn merges_keep_every_name_and_resolve_old_ids(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    people: &People,
) -> anyhow::Result<()> {
    let by_old_name = resolve(neomem, proof, "Samuel Kim")?;
    let by_old_id = resolve(neomem, proof, &people.samuel.to_string())?;
    let sam = view(neomem, proof, "Sam Kim")?;
    proof.claim(
        "merge_entities keeps every name as an alias, records merged_from, and the old name and ID resolve to the survivor",
        by_old_name == vec![people.sam]
            && by_old_id == vec![people.sam]
            && sam.as_ref().is_some_and(|view| view.entity.aliases().iter().any(|alias| alias == "Samuel Kim") && view.entity.merged_from() == [people.samuel]),
        format!("{by_old_name:?}"),
    );
    proof.claim(
        "the survivor now lists the facts that were about the duplicate",
        sam.is_some_and(|view| view.facts.iter().any(|fact| fact.content().contains("sam@acme"))),
        "",
    );
    let duplicate = match neomem.call(proof, Request::Show { id: people.samuel.into() })? {
        Response::Shown(shown) => shown.text,
        _ => String::new(),
    };
    proof.claim(
        "the duplicate's file is kept, marked merged into the survivor",
        duplicate.contains(&format!("merged_into: {}", people.sam)),
        "",
    );
    Ok(())
}

fn aliases_and_links_land(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let edge = resolve(neomem, proof, "edge app")?;
    proof.claim("add_alias makes the new name resolve", edge.len() == 1, format!("{edge:?}"));
    let paywall = view(neomem, proof, "Paywall")?;
    let acme_ids = resolve(neomem, proof, "Acme Labs")?;
    let acme = view(neomem, proof, "Acme Labs")?;
    proof.claim(
        "link_entity records part_of, and possibly_same stays a weak link rather than a merge",
        paywall.is_some_and(|view| {
            view.linked.iter().any(|link| link.kind == LinkKind::PartOf && link.entity.name() == "Edge")
        }) && acme_ids.len() == 1
            && acme.is_some_and(|view| view.linked.iter().any(|link| link.kind == LinkKind::PossiblySame)),
        "",
    );
    Ok(())
}

fn recurring_names_become_entities_citing_facts(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    people: &People,
) -> anyhow::Result<()> {
    let vega = view(neomem, proof, "Vega")?;
    let orion = resolve(neomem, proof, "Orion")?;
    proof.claim(
        "create_entity makes an entity for a recurring name and links the cited facts to it",
        vega.is_some_and(|view| view.facts.iter().any(|fact| fact.id() == people.vega_fact)),
        "",
    );
    proof.claim("an entity that cites no fact is not created", orion.is_empty(), "");
    Ok(())
}

fn the_keeper_is_limited(proof: &mut ProofRun, report: Option<&DreamReport>) {
    proof.claim(
        "an invented entity number, a creation without facts and a fact-writing tool are all rejected, within 8 rounds",
        report.is_some_and(|report| report.entity_keeper.rejected == 3 && report.entity_keeper.rounds <= 8 && report.entity_keeper.writes == 5),
        format!("{:?}", report.map(|report| report.entity_keeper)),
    );
}
