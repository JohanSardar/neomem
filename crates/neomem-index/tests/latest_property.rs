#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use neomem_core::{
    Durability, EntityId, EpochMs, Fact, FactId, FactLink, FactLinkKind, Importance, IntakeId, Memory,
    NewFact,
};
use neomem_index::{Change, Index};
use proptest::prelude::*;

const DAY: i64 = 86_400_000;

fn version(chain: u128, number: i64, previous: Option<FactId>, replaced: bool) -> Fact {
    let valid_at = EpochMs::from_millis(1_700_000_000_000 + number * DAY);
    let mut fact = Fact::new(NewFact {
        id: FactId::from_parts(valid_at, chain * 1000 + u128::try_from(number).unwrap()),
        content: format!("chain {chain} version {number}"),
        tags: vec![],
        about: vec![EntityId::from_parts(EpochMs::from_millis(1), chain)],
        recorded_at: valid_at,
        valid_at,
        importance: Importance::new(5).unwrap(),
        durability: Durability::Ordinary,
        sources: vec![IntakeId::from_parts(valid_at, 1)],
        links: previous.map(|old| FactLink { kind: FactLinkKind::Updates, to: old }).into_iter().collect(),
    })
    .unwrap();
    if replaced {
        fact.invalidate(EpochMs::from_millis(valid_at.as_millis() + DAY), valid_at);
    }
    fact
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_version_of_every_chain_resolves_to_its_newest(
        lengths in proptest::collection::vec(2_usize..=10, 1..6),
        arrival in any::<u64>(),
    ) {
        let mut chains: Vec<Vec<Fact>> = Vec::new();
        for (index, length) in lengths.iter().enumerate() {
            let chain = u128::try_from(index).unwrap() + 1;
            let mut versions: Vec<Fact> = Vec::new();
            for number in 1..=i64::try_from(*length).unwrap() {
                let previous = versions.last().map(Fact::id);
                versions.push(version(chain, number, previous, number < i64::try_from(*length).unwrap()));
            }
            chains.push(versions);
        }
        let mut changes: Vec<Change> = chains
            .iter()
            .flatten()
            .map(|fact| Change { memory: Memory::Fact(fact.clone()), path: PathBuf::from("x.md"), vector: None })
            .collect();
        let mut seed = arrival;
        for index in (1..changes.len()).rev() {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            changes.swap(index, usize::try_from(seed % (index as u64 + 1)).unwrap());
        }
        let directory = tempfile::tempdir().unwrap();
        let index = Index::open(&directory.path().join("index.db"), None).unwrap().index;
        for change in changes {
            index.apply(&[change]).unwrap();
        }
        let snapshot = index.snapshot();
        for versions in &chains {
            let newest = versions.last().unwrap().id();
            for fact in versions {
                prop_assert_eq!(snapshot.latest(fact.id()), newest);
            }
        }
    }
}
