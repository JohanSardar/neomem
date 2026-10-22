use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, ensure};
use clap::Args;
use neomem_proto::SearchResults;
use serde_json::Value;

use crate::data;
use crate::model::Settings;
use crate::report::{self, Item};
use crate::runner;

#[derive(Debug, Args)]
pub struct Options {
    pub run: PathBuf,
    #[arg(long)]
    pub dataset: PathBuf,
    #[arg(long)]
    pub models: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
}

pub fn run(options: &Options) -> anyhow::Result<()> {
    ensure!(!options.out.exists(), "output directory already exists");
    let settings: Settings = serde_json::from_slice(&std::fs::read(&options.models)?)?;
    let (questions, checksum) = data::load(&options.dataset)?;
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(options.run.join("manifest.json"))?)?;
    ensure!(manifest["dataset_sha256"] == checksum.as_str(), "the run used a different dataset");
    let earlier: Vec<Item> = serde_json::from_slice(&std::fs::read(options.run.join("items.json"))?)?;
    let questions: BTreeMap<String, data::BenchmarkItem> =
        questions.into_iter().map(|question| (question.question_id.clone(), question)).collect();
    manifest["rescored_from"] = Value::String(options.run.display().to_string());
    manifest["models"] = serde_json::to_value(&settings)?;
    manifest["reader_prompt"] = Value::String(include_str!("../prompts/reader.md").to_owned());
    manifest["judge_prompts"] = Value::String(include_str!("../prompts/judge.json").to_owned());
    std::fs::create_dir_all(&options.out)?;
    std::fs::write(options.out.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    let mut items = Vec::new();
    for item in earlier {
        let question = questions.get(&item.id).context("a rescored question is missing from the dataset")?;
        eprintln!("{} {}", question.question_id, question.category());
        let results: Option<SearchResults> = serde_json::from_value(item.results.clone())?;
        let answered = runner::answer(question, results.as_ref(), Instant::now(), &settings)?;
        items.push(Item {
            correct: answered.correct,
            latency_ms: answered.latency_ms,
            context_tokens: answered.context_tokens,
            answer: answered.answer,
            judge_answer: answered.judge_answer,
            ..item
        });
        std::fs::write(options.out.join("items.json"), serde_json::to_vec_pretty(&items)?)?;
    }
    report::write(&options.out)
}
