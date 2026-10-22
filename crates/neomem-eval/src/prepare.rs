use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::ensure;
use clap::Args;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::data::{self, BenchmarkItem, History, SharedDataset, SharedQuestion};

#[derive(Debug, Args)]
pub struct Options {
    pub dataset: PathBuf,
    #[arg(long, default_value_t = 60)]
    pub sample: usize,
    #[arg(long, default_value_t = 20_261_021)]
    pub seed: u64,
    #[arg(long)]
    pub held_out: bool,
    #[arg(long, default_value_t = 1)]
    pub shards: usize,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Split {
    pub dataset_sha256: String,
    pub seed: u64,
    pub held_out: bool,
    pub shard: usize,
    pub shards: usize,
    pub selected: Vec<String>,
    pub other: Vec<String>,
}

pub fn select(
    questions: Vec<BenchmarkItem>,
    sample: usize,
    seed: u64,
    held_out: bool,
) -> (Vec<BenchmarkItem>, Vec<String>) {
    let (development, reserved) = data::split(questions, sample, seed);
    let (mut selected, other) = if held_out { (reserved, development) } else { (development, reserved) };
    selected.sort_by(|first, second| first.haystack_session_ids.cmp(&second.haystack_session_ids));
    (selected, other.into_iter().map(|item| item.question_id).collect())
}

pub fn run(options: &Options) -> anyhow::Result<()> {
    ensure!(options.shards > 0, "shards must be positive");
    ensure!(!options.out.exists(), "output directory already exists");
    let (questions, checksum) = data::load(&options.dataset)?;
    let (selected, other) = select(questions, options.sample, options.seed, options.held_out);
    ensure!(!selected.is_empty(), "empty sample");
    let mut split = Split {
        dataset_sha256: checksum,
        seed: options.seed,
        held_out: options.held_out,
        shard: 0,
        shards: options.shards,
        selected: selected.iter().map(|item| item.question_id.clone()).collect(),
        other,
    };
    std::fs::create_dir_all(&options.out)?;
    std::fs::write(options.out.join("split.json"), serde_json::to_vec_pretty(&split)?)?;
    for shard in 0..options.shards {
        split.shard = shard;
        let questions = crate::runner::shard_of(selected.clone(), shard, options.shards);
        let dataset = compact(questions, split.clone())?;
        let file = std::fs::File::create(options.out.join(format!("shard-{shard}.json")))?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer(&mut writer, &dataset)?;
        writer.flush()?;
    }
    Ok(())
}

fn compact(questions: Vec<BenchmarkItem>, split: Split) -> anyhow::Result<SharedDataset> {
    let mut dataset = SharedDataset {
        format: "neomem-eval/2".to_owned(),
        sessions: BTreeMap::new(),
        histories: BTreeMap::new(),
        questions: Vec::new(),
        prepared: Some(split),
    };
    for mut item in questions {
        let mut keys = Vec::new();
        for (id, turns) in item.haystack_session_ids.iter().zip(item.haystack_sessions.iter()) {
            let key = if dataset.sessions.get(id).is_some_and(|existing| existing != turns) {
                let mut key = format!("{id}~");
                for byte in Sha256::digest(serde_json::to_vec(turns)?) {
                    write!(&mut key, "{byte:02x}")?;
                }
                key
            } else {
                id.clone()
            };
            if let Some(existing) = dataset.sessions.get(&key) {
                ensure!(existing == turns, "session key collision: {key}");
            }
            dataset.sessions.entry(key.clone()).or_insert_with(|| turns.clone());
            keys.push(key);
        }
        let session_keys = (keys != *item.haystack_session_ids).then(|| Arc::new(keys));
        let history = History {
            session_ids: item.haystack_session_ids.clone(),
            dates: item.haystack_dates.clone(),
            session_keys,
        };
        let id = item.history_id.as_ref().map_or_else(
            || {
                dataset
                    .histories
                    .iter()
                    .find(|(_, value)| {
                        value.session_ids == history.session_ids
                            && value.dates == history.dates
                            && value.session_keys == history.session_keys
                    })
                    .map_or_else(|| dataset.histories.len().to_string(), |(id, _)| id.clone())
            },
            |id| id.as_ref().clone(),
        );
        dataset.histories.entry(id.clone()).or_insert(history);
        item.haystack_sessions = Arc::default();
        item.haystack_session_ids = Arc::default();
        item.haystack_dates = Arc::default();
        dataset.questions.push(SharedQuestion { item, history: id });
    }
    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<BenchmarkItem> {
        serde_json::from_str(include_str!(
            "../../../.agents/skills/neomem-control/fixtures/eval/questions.json"
        ))
        .unwrap()
    }

    #[test]
    fn shared_format_preserves_histories_and_shares_storage() {
        let original = fixture();
        let mut repeated = original.clone();
        let mut duplicate = original[0].clone();
        duplicate.question_id = "duplicate".to_owned();
        repeated.push(duplicate);
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.json");
        std::fs::write(&source, serde_json::to_vec(&repeated).unwrap()).unwrap();
        let out = directory.path().join("prepared");
        run(&Options {
            dataset: source,
            sample: usize::MAX,
            seed: 7,
            held_out: false,
            shards: 1,
            out: out.clone(),
        })
        .unwrap();
        let (loaded, _, split) = data::load_with_split(&out.join("shard-0.json")).unwrap();
        assert!(split.is_some());
        for question in &loaded {
            let before = repeated.iter().find(|item| item.question_id == question.question_id).unwrap();
            assert!(before.shares_history_with(question));
            assert_eq!(before.evidence_sessions(), question.evidence_sessions());
        }
        let first = loaded.iter().find(|item| item.question_id == original[0].question_id).unwrap();
        let duplicate = loaded.iter().find(|item| item.question_id == "duplicate").unwrap();
        assert!(first.shares_history_with(duplicate));
        assert!(Arc::ptr_eq(&first.haystack_sessions, &duplicate.haystack_sessions));
        assert!(Arc::ptr_eq(&first.haystack_dates, &duplicate.haystack_dates));
    }

    #[test]
    fn prepared_shards_preserve_direct_selection_and_identity() {
        for held_out in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("source.json");
            std::fs::write(&source, serde_json::to_vec(&fixture()).unwrap()).unwrap();
            let (_, checksum) = data::load(&source).unwrap();
            let out = directory.path().join("prepared");
            run(&Options { dataset: source, sample: 2, seed: 17, held_out, shards: 3, out: out.clone() })
                .unwrap();
            let (selected, other) = select(fixture(), 2, 17, held_out);
            for shard in 0..3 {
                let (loaded, _, split) =
                    data::load_with_split(&out.join(format!("shard-{shard}.json"))).unwrap();
                let ids = |items: Vec<BenchmarkItem>| {
                    items.into_iter().map(|item| item.question_id).collect::<Vec<_>>()
                };
                assert_eq!(ids(loaded), ids(crate::runner::shard_of(selected.clone(), shard, 3)));
                let split = split.unwrap();
                assert_eq!(split.dataset_sha256, checksum);
                assert_eq!(split.other, other);
                assert_eq!(split.shard, shard);
                assert_eq!(split.held_out, held_out);
            }
        }
    }
}
