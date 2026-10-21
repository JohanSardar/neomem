use neomem_core::{EpochMs, ExternalLink, ExternalLinkKind, STARTING_TAGS, Tag};
use neomem_state::StateDb;

#[test]
fn registry_is_seeded_persistent_and_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let state = StateDb::open(&path).unwrap();
    assert_eq!(state.tags().unwrap().len(), STARTING_TAGS.len());
    for tag in STARTING_TAGS {
        assert!(state.tags().unwrap().contains(&Tag::new(tag).unwrap()));
    }
    let tag = Tag::new("astronomy").unwrap();
    state.register_tag(&tag).unwrap();
    state.register_tag(&tag).unwrap();
    let link = ExternalLink {
        external: "ext:episodes/epi_123".parse().unwrap(),
        to: "ent_01M56PGSJG83SYHRJXJG7DJ13N".parse().unwrap(),
        kind: ExternalLinkKind::Mentions,
        recorded_at: EpochMs::from_millis(1),
    };
    state.register_external_link(&link).unwrap();
    state.register_external_link(&link).unwrap();
    drop(state);
    let reopened = StateDb::open(&path).unwrap();
    assert_eq!(reopened.tags().unwrap().len(), STARTING_TAGS.len() + 1);
    assert_eq!(reopened.external_links().unwrap(), vec![link]);
}
