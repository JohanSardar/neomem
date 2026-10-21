use std::fs;
use std::path::Path;

use neomem_core::{EntityId, EpochMs, FactId, IntakeId, LinkKind};
use neomem_proto::{Request, Response, SearchHit, SearchQuery};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;
use crate::scenarios::support;

const START: i64 = 1_785_000_000_000;
const QUESTION: &str = "how are Edge release builds signed";

struct Linked {
    seed: FactId,
    replaced: FactId,
    replacement: FactId,
    contradiction: FactId,
    off_topic: FactId,
}

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let neomem = IsolatedNeomem::start(binaries)?;
    let linked = write_linked_facts(neomem.home().root())?;
    support::wait_for_index(&neomem, proof, 10, 2)?;
    let hits = search(
        &neomem,
        proof,
        SearchQuery { k: Some(2), text: QUESTION.to_owned(), ..SearchQuery::default() },
    )?;
    let Some(seed) = hits.first().filter(|hit| hit.fact.id() == linked.seed) else {
        proof.claim(
            "the seed fact answers the question",
            false,
            format!("{:?}", hits.first().map(|hit| hit.fact.content())),
        );
        return Ok(());
    };
    linked_facts_hang_under_their_seed(proof, seed, &linked);
    let weakest = hits.iter().map(|hit| hit.score).fold(f32::INFINITY, f32::min);
    weak_links_are_kept_out(proof, seed, &linked, weakest);
    the_entities_come_with_names(proof, seed);
    no_model_besides_the_embedder(proof, &neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn id(seed: u128) -> FactId {
    FactId::from_parts(EpochMs::from_millis(START + i64::try_from(seed).unwrap_or(0) * 60_000), seed)
}

fn write_linked_facts(home: &Path) -> anyhow::Result<Linked> {
    let edge = EntityId::from_parts(EpochMs::from_millis(START), 1);
    let sam = EntityId::from_parts(EpochMs::from_millis(START), 2);
    fs::create_dir_all(home.join("entities/project"))?;
    fs::create_dir_all(home.join("entities/person"))?;
    for (entity, kind, name) in [(edge, "project", "Edge"), (sam, "person", "Sam")] {
        fs::write(
            home.join(format!("entities/{kind}/{}.md", name.to_lowercase())),
            format!(
                "---\nid: {entity}\nkind: entity\nentity_kind: {kind}\nname: {name}\naliases: []\nrecorded_at: {START}\nupdated_at: {START}\nmerged_from: []\nmerged_into: null\nrelations: []\nlinks: []\n---\n"
            ),
        )?;
    }
    let linked =
        Linked { seed: id(1), replaced: id(2), replacement: id(3), contradiction: id(7), off_topic: id(8) };
    let now = now_ms();
    let facts: [FactFile<'_>; 10] = [
        (
            linked.seed,
            "Edge release builds are signed with the distribution certificate.",
            "[]",
            false,
            &[edge],
            now,
        ),
        (
            id(10),
            "How Edge release builds are signed was asked in the old wiki.",
            "[]",
            false,
            &[edge],
            START,
        ),
        (
            linked.replaced,
            "The Edge distribution certificate lives in Johan's login keychain.",
            &extends(linked.seed),
            true,
            &[edge],
            now,
        ),
        (
            linked.replacement,
            "The Edge distribution certificate lives in the team keychain on the build Mac.",
            &format!("[{{type: updates, to: {}}}]", linked.replaced),
            false,
            &[edge],
            now,
        ),
        (
            id(4),
            "The Edge distribution certificate is rotated every October.",
            &extends(linked.seed),
            false,
            &[edge],
            now,
        ),
        (
            id(5),
            "The Edge distribution certificate needs the account owner's approval.",
            &extends(linked.seed),
            false,
            &[edge],
            now,
        ),
        (
            id(6),
            "Edge certificate handling is part of the Xcode Cloud pipeline.",
            &format!("[{{type: part_of, to: {}}}]", linked.seed),
            false,
            &[edge],
            now,
        ),
        (
            linked.contradiction,
            "Sam claims Edge builds use an ad hoc certificate.",
            &format!("[{{type: contradicts, to: {}}}]", linked.seed),
            false,
            &[edge, sam],
            now,
        ),
        (linked.off_topic, "It rained in Stockholm all week.", &extends(linked.seed), false, &[edge], now),
        (id(9), "Sam prefers green tea.", "[]", false, &[sam], now),
    ];
    fs::create_dir_all(home.join("facts/2026/07"))?;
    for (fact, content, links, replaced, about, recorded) in facts {
        let about: Vec<String> = about.iter().map(ToString::to_string).collect();
        let invalid = if replaced { (recorded - 3_600_000).to_string() } else { "null".to_owned() };
        let valid = recorded - 7_200_000;
        let source = IntakeId::from_parts(EpochMs::from_millis(START), 1);
        fs::write(
            home.join(format!("facts/2026/07/{fact}_linked.md")),
            format!(
                "---\nid: {fact}\nkind: fact\ntags: [code]\nabout: [{}]\nrecorded_at: {recorded}\nupdated_at: {recorded}\nvalid_at: {valid}\ninvalid_at: {invalid}\nimportance: 5\ndurability: ordinary\nsources: [{source}]\nlinks: {links}\nevidence: []\n---\n{content}\n",
                about.join(", ")
            ),
        )?;
    }
    Ok(linked)
}

type FactFile<'text> = (FactId, &'text str, &'text str, bool, &'text [EntityId], i64);

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(START)
}

fn extends(to: FactId) -> String {
    format!("[{{type: extends, to: {to}}}]")
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

fn linked_facts_hang_under_their_seed(proof: &mut ProofRun, seed: &SearchHit, linked: &Linked) {
    let kinds: Vec<String> = seed.linked.iter().map(|link| link.kind.to_string()).collect();
    proof.claim(
        "linked facts come underneath their seed, at most 2, with their link type and usefulness",
        seed.linked.len() == 2
            && seed.linked.iter().any(|link| matches!(link.kind, LinkKind::Extends | LinkKind::PartOf)),
        format!("{kinds:?}"),
    );
    proof.claim(
        "a contradiction is always shown, flagged, however weak",
        seed.conflict
            && seed
                .linked
                .iter()
                .any(|link| link.kind == LinkKind::Contradicts && link.fact.id() == linked.contradiction),
        "",
    );
    let shown: Vec<String> = seed.linked.iter().map(|link| link.fact.id().to_string()).collect();
    proof.claim(
        "a linked fact is swapped for its newest version; the replaced one is never shown",
        !shown.contains(&linked.replaced.to_string()),
        format!("{shown:?} replacement {}", linked.replacement),
    );
}

fn weak_links_are_kept_out(proof: &mut ProofRun, seed: &SearchHit, linked: &Linked, weakest: f32) {
    let threshold = 0.8 * weakest;
    let weak = seed
        .linked
        .iter()
        .filter(|link| link.kind != LinkKind::Contradicts)
        .any(|link| link.usefulness < threshold);
    proof.claim(
        "every other kept link scores at least 0.8 × the weakest seed, and an off-topic extension is dropped",
        !weak && !seed.linked.iter().any(|link| link.fact.id() == linked.off_topic),
        format!(
            "threshold {threshold:.3}; {:?}",
            seed.linked.iter().map(|link| link.usefulness).collect::<Vec<_>>()
        ),
    );
}

fn the_entities_come_with_names(proof: &mut ProofRun, seed: &SearchHit) {
    proof.claim(
        "the entities a seed is about come with it, by name",
        seed.entities.iter().any(|entity| entity.name == "Edge"),
        format!("{:?}", seed.entities.iter().map(|entity| entity.name.as_str()).collect::<Vec<_>>()),
    );
}

fn no_model_besides_the_embedder(proof: &mut ProofRun, neomem: &IsolatedNeomem) -> anyhow::Result<()> {
    let routes: Vec<String> = neomem.fake_model_calls()?.into_iter().map(|call| call.route).collect();
    proof.claim(
        "scoring linked memories calls no model besides the embedder",
        routes.iter().all(|route| route == "/v1/embeddings"),
        format!("{} calls", routes.len()),
    );
    Ok(())
}
