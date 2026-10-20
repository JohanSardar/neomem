#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use neomem_core::{
    Durability, Embedder, Entity, EntityId, EpochMs, Fact, FactId, FactLink, FactLinkKind, Filter,
    Importance, IntakeId, Label, Memory, NewEntity, NewFact, Purpose, Tag,
};
use neomem_index::{Change, Index};
use neomem_testkit::HashEmbedder;

fn at(millis: i64) -> EpochMs {
    EpochMs::from_millis(millis)
}

fn entity(seed: u128, name: &str, aliases: &[&str]) -> Entity {
    Entity::new(NewEntity {
        id: EntityId::from_parts(at(1), seed),
        kind: Label::new("project").unwrap(),
        name: name.to_owned(),
        aliases: aliases.iter().map(ToString::to_string).collect(),
        description: String::new(),
        recorded_at: at(1),
    })
    .unwrap()
}

fn fact(
    seed: u128,
    content: &str,
    about: EntityId,
    recorded: i64,
    updates: Option<FactId>,
    tags: &[&str],
) -> Fact {
    Fact::new(NewFact {
        id: FactId::from_parts(at(recorded), seed),
        content: content.to_owned(),
        tags: tags.iter().map(|tag| Tag::new(tag).unwrap()).collect(),
        about: vec![about],
        recorded_at: at(recorded),
        valid_at: at(recorded),
        importance: Importance::new(5).unwrap(),
        durability: Durability::Ordinary,
        sources: vec![IntakeId::from_parts(at(recorded), seed)],
        links: updates.map(|old| FactLink { kind: FactLinkKind::Updates, to: old }).into_iter().collect(),
    })
    .unwrap()
}

fn change(memory: Memory) -> Change {
    let vector = HashEmbedder.embed(&[&memory.embedding_text()], Purpose::Document).unwrap().pop();
    Change { path: PathBuf::from(format!("{}.md", memory.id())), memory, vector }
}

struct World {
    edge: Entity,
    maya: Entity,
    october: Fact,
    november: Fact,
    december: Fact,
    sister: Fact,
}

fn world() -> World {
    let edge = entity(1, "Edge", &["Edge app"]);
    let maya = entity(2, "Maya", &[]);
    let october = fact(10, "Edge ships on October 15", edge.id(), 100, None, &["code"]);
    let november = fact(11, "Edge ships on November 1", edge.id(), 200, Some(october.id()), &["code"]);
    let december = fact(12, "Edge ships on December 3", edge.id(), 300, Some(november.id()), &["code"]);
    let sister = fact(13, "Maya is my sister", maya.id(), 150, None, &["social"]);
    World { edge, maya, october, november, december, sister }
}

fn changes(world: &World) -> Vec<Change> {
    vec![
        change(Memory::Entity(world.edge.clone())),
        change(Memory::Entity(world.maya.clone())),
        change(Memory::Fact(world.october.clone())),
        change(Memory::Fact(world.november.clone())),
        change(Memory::Fact(world.december.clone())),
        change(Memory::Fact(world.sister.clone())),
    ]
}

fn open(directory: &tempfile::TempDir) -> neomem_index::Opened {
    Index::open(&directory.path().join("index.db"), Some((HashEmbedder.model_id(), HashEmbedder.dims())))
        .unwrap()
}

#[test]
fn every_version_in_a_chain_points_at_the_newest() {
    let directory = tempfile::tempdir().unwrap();
    let index = open(&directory).index;
    let world = world();
    index.apply(&changes(&world)).unwrap();
    let snapshot = index.snapshot();
    assert_eq!(snapshot.latest(world.october.id()), world.december.id());
    assert_eq!(snapshot.latest(world.november.id()), world.december.id());
    assert_eq!(snapshot.latest(world.december.id()), world.december.id());
    assert_eq!(snapshot.latest(world.sister.id()), world.sister.id());
}

#[test]
fn a_snapshot_never_changes_after_it_was_taken() {
    let directory = tempfile::tempdir().unwrap();
    let index = open(&directory).index;
    let world = world();
    index.apply(&changes(&world)[..4]).unwrap();
    let before = index.snapshot();
    index.apply(&changes(&world)[4..]).unwrap();
    assert_eq!(before.counts().facts, 2);
    assert_eq!(before.latest(world.october.id()), world.november.id());
    assert_eq!(index.snapshot().latest(world.october.id()), world.december.id());
}

#[test]
fn reopening_loads_the_same_index_without_re_embedding() {
    let directory = tempfile::tempdir().unwrap();
    let world = world();
    let counts = {
        let opened = open(&directory);
        assert!(opened.needs_rebuild);
        opened.index.apply(&changes(&world)).unwrap();
        opened.index.counts()
    };
    let reopened = open(&directory);
    assert!(!reopened.needs_rebuild);
    assert_eq!(reopened.missing_vectors, Vec::new());
    assert_eq!(reopened.index.counts(), counts);
    assert_eq!(reopened.index.snapshot().latest(world.october.id()), world.december.id());
}

#[test]
fn a_rebuild_matches_incremental_applies() {
    let directory = tempfile::tempdir().unwrap();
    let world = world();
    let index = open(&directory).index;
    for single in changes(&world) {
        index.apply(&[single]).unwrap();
    }
    let incremental = index.counts();
    let rebuilt = index.rebuild(&changes(&world)).unwrap();
    assert_eq!(incremental, rebuilt);
}

#[test]
fn names_and_aliases_resolve_case_insensitively() {
    let directory = tempfile::tempdir().unwrap();
    let world = world();
    let index = open(&directory).index;
    index.apply(&changes(&world)).unwrap();
    let snapshot = index.snapshot();
    assert_eq!(snapshot.resolve("edge APP"), vec![world.edge.id()]);
    assert_eq!(snapshot.resolve("nobody"), Vec::<EntityId>::new());
}

#[test]
fn a_merged_entity_resolves_to_its_survivor() {
    let directory = tempfile::tempdir().unwrap();
    let world = world();
    let index = open(&directory).index;
    let mut duplicate = entity(3, "Edge iOS", &[]);
    let mut survivor = world.edge.clone();
    survivor.absorb(&duplicate, at(500)).unwrap();
    duplicate.retire_into(survivor.id(), at(500)).unwrap();
    index
        .apply(&[change(Memory::Entity(survivor.clone())), change(Memory::Entity(duplicate.clone()))])
        .unwrap();
    let snapshot = index.snapshot();
    assert_eq!(snapshot.survivor(duplicate.id()), survivor.id());
    assert_eq!(snapshot.resolve("Edge iOS"), vec![survivor.id()]);
}

#[test]
fn nearest_facts_respect_tags_entities_and_hidden_tags() {
    let directory = tempfile::tempdir().unwrap();
    let world = world();
    let index = open(&directory).index;
    index.apply(&changes(&world)).unwrap();
    let raw_note = fact(14, "Edge ships raw note", world.edge.id(), 400, None, &["raw"]);
    index.apply(&[change(Memory::Fact(raw_note.clone()))]).unwrap();
    let snapshot = index.snapshot();
    let query = HashEmbedder::vector("when does Edge ship");
    let unfiltered = snapshot.nearest_facts(&query, &Filter::default(), 10);
    assert!(unfiltered.iter().all(|(id, _)| *id != raw_note.id()), "raw is hidden by default");
    assert!(unfiltered.iter().any(|(id, _)| *id == world.october.id()), "old versions are still seeds");
    let about_maya = Filter { entities: vec![world.maya.id()], ..Filter::default() };
    let found = snapshot.nearest_facts(&query, &about_maya, 10);
    assert_eq!(found.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![world.sister.id()]);
    let social = Filter { tags: vec![Tag::new("social").unwrap()], ..Filter::default() };
    assert_eq!(snapshot.nearest_facts(&query, &social, 10).len(), 1);
}
