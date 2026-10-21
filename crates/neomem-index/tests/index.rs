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

#[test]
fn a_partial_swap_resumes_and_publishes_one_complete_snapshot() {
    use neomem_core::{ModelId, Vector};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.db");
    let index = open(&directory).index;
    let world = world();
    index.apply(&changes(&world)).unwrap();
    let old = index.snapshot();
    let query = HashEmbedder::vector("when Edge ships");
    let old_answer = old.nearest_facts(&query, &Filter::default(), 6);
    let target = ModelId::new("new-three-dimensional-model");
    let mut first = index.pending_vectors(&target, 2).unwrap();
    for change in &mut first {
        change.vector = Some(Vector::normalized(vec![1.0, 0.0, 0.0]));
    }
    index.stage_vectors(&target, &first).unwrap();
    assert!(index.activate_model(&target, 3).is_err());
    assert_eq!(index.snapshot().model(), old.model());
    drop(index);
    let index = Index::open(&path, Some((target.clone(), 3))).unwrap().index;
    assert_eq!(index.snapshot().model(), old.model());
    let mut remaining = index.pending_vectors(&target, 32).unwrap();
    assert_eq!(remaining.len(), 4);
    for change in &mut remaining {
        change.vector = Some(Vector::normalized(vec![1.0, 0.0, 0.0]));
    }
    index.stage_vectors(&target, &remaining).unwrap();
    index.activate_model(&target, 3).unwrap();
    assert_eq!(index.snapshot().model(), Some(&target));
    assert_eq!(index.snapshot().counts().vectors, 6);
    assert_eq!(old.model(), Some(&HashEmbedder.model_id()));
    assert_eq!(old.counts().vectors, 6);
    assert_eq!(old.nearest_facts(&query, &Filter::default(), 6), old_answer);
    assert_eq!(
        index.snapshot().nearest_facts(&Vector::normalized(vec![1.0, 0.0, 0.0]), &Filter::default(), 6).len(),
        4
    );
    let connection = rusqlite::Connection::open(&path).unwrap();
    let old_rows: i64 = connection
        .query_row("SELECT count(*) FROM vectors WHERE model_id != ?1", [target.as_str()], |row| row.get(0))
        .unwrap();
    assert_eq!(old_rows, 0);
    drop(index);
    let reopened = Index::open(&path, Some((HashEmbedder.model_id(), HashEmbedder.dims()))).unwrap();
    assert_eq!(reopened.index.snapshot().model(), Some(&target));
}

#[test]
fn edits_during_a_swap_invalidate_the_pending_vector() {
    use neomem_core::{ModelId, Vector};

    for model in [None, Some((HashEmbedder.model_id(), HashEmbedder.dims()))] {
        let directory = tempfile::tempdir().unwrap();
        let index = Index::open(&directory.path().join("index.db"), model).unwrap().index;
        let world = world();
        index.apply(&changes(&world)).unwrap();
        let target = ModelId::new("pending");
        let mut pending = index.pending_vectors(&target, 32).unwrap();
        for change in &mut pending {
            change.vector = Some(Vector::normalized(vec![1.0, 0.0]));
        }
        index.stage_vectors(&target, &pending).unwrap();
        assert!(index.pending_vectors(&target, 32).unwrap().is_empty());
        let edited =
            fact(11, "Edge ships on November 2", world.edge.id(), 200, Some(world.october.id()), &["code"]);
        index.apply(&[change(Memory::Fact(edited))]).unwrap();
        assert_eq!(index.pending_vectors(&target, 32).unwrap().len(), 1);
        assert!(index.activate_model(&target, 2).is_err());
    }
}

#[test]
fn a_restart_finishes_cleanup_after_an_interrupted_activation() {
    use neomem_core::{ModelId, Vector};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.db");
    let index = open(&directory).index;
    index.apply(&changes(&world())).unwrap();
    let target = ModelId::new("new-model");
    let mut pending = index.pending_vectors(&target, 32).unwrap();
    for change in &mut pending {
        change.vector = Some(Vector::normalized(vec![1.0, 0.0]));
    }
    index.stage_vectors(&target, &pending).unwrap();
    index.activate_model(&target, 2).unwrap();
    drop(index);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("INSERT INTO vectors SELECT memory_id, 'old-cache', data FROM vectors; UPDATE active_model SET cleanup = 1;").unwrap();
    drop(connection);
    let reopened = Index::open(&path, Some((target.clone(), 2))).unwrap();
    assert_eq!(reopened.index.snapshot().model(), Some(&target));
    let connection = rusqlite::Connection::open(&path).unwrap();
    let old_rows: i64 = connection
        .query_row("SELECT count(*) FROM vectors WHERE model_id = 'old-cache'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(old_rows, 0);
}
