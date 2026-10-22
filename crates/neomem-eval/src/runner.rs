use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, ensure};
use clap::{Args, ValueEnum};
use neomem_proto::{Request, Response, SearchQuery, SearchResults};
use serde::Serialize;
use serde_json::{Value, json};

use crate::context;
use crate::daemon::Daemon;
use crate::data::{self, BenchmarkItem};
use crate::model::Settings;
use crate::report::{self, Item};

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arm {
    Search,
    MemoryOff,
}

#[derive(Debug, Args)]
pub struct Options {
    pub dataset: PathBuf,
    #[arg(long, default_value_t = 60)]
    pub sample: usize,
    #[arg(long, default_value_t = 20_261_021)]
    pub seed: u64,
    #[arg(long, value_enum, default_value_t = Arm::Search)]
    pub arm: Arm,
    #[arg(long)]
    pub models: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
    #[arg(long)]
    pub held_out: bool,
    #[arg(long, default_value_t = 0)]
    pub shard: usize,
    #[arg(long, default_value_t = 1)]
    pub shards: usize,
    #[arg(long)]
    pub resume: bool,
    #[arg(long, conflicts_with_all = ["sample", "seed", "held_out", "shard", "shards"])]
    pub prepared: bool,
    #[command(flatten)]
    pub inspection: Inspection,
}

#[derive(Debug, Args)]
pub struct Inspection {
    #[arg(long)]
    pub inspect: bool,
}

struct Ingested {
    history_of: BenchmarkItem,
    daemon: Daemon,
    evidence: BTreeMap<String, String>,
    ingestion_ms: f64,
}

pub fn run(options: &Options) -> anyhow::Result<()> {
    ensure!(
        options.inspection.inspect || options.resume || !options.out.exists(),
        "output directory already exists"
    );
    ensure!(options.shard < options.shards, "the shard must be below the shard count");
    let (questions, mut checksum, prepared) = data::load_with_split(&options.dataset)?;
    let (sample, other, seed, held_out, shard, shards) = if options.prepared {
        let split = prepared.context("dataset is not a prepared shard")?;
        checksum = split.dataset_sha256;
        (questions, split.other, split.seed, split.held_out, split.shard, split.shards)
    } else {
        let (sample, other) =
            crate::prepare::select(questions, options.sample, options.seed, options.held_out);
        ensure!(!sample.is_empty(), "empty sample");
        (
            shard_of(sample, options.shard, options.shards),
            other,
            options.seed,
            options.held_out,
            options.shard,
            options.shards,
        )
    };
    if options.inspection.inspect {
        println!(
            "{}",
            serde_json::to_string(&sample.iter().map(|item| &item.question_id).collect::<Vec<_>>())?
        );
        return Ok(());
    }
    let settings: Settings = serde_json::from_slice(&std::fs::read(&options.models)?)?;
    std::fs::create_dir_all(&options.out)?;
    let split = if held_out { "held-out" } else { "development" };
    let manifest = json!({"dataset_sha256":checksum,"seed":seed,"arm":options.arm,"split":split,
        "shard":shard,"shards":shards,
        "selected":sample.iter().map(|q| &q.question_id).collect::<Vec<_>>(),
        "held_out":other,
        "models":settings,"k":10,"timezone":"UTC","reader_prompt":include_str!("../prompts/reader.md"),
        "judge_prompts":include_str!("../prompts/judge.json"),"git_revision":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().map(|o|String::from_utf8_lossy(&o.stdout).trim().to_owned())});
    std::fs::write(options.out.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    let earlier = options.out.join("items.json");
    let mut items: Vec<Item> = if options.resume && earlier.exists() {
        serde_json::from_slice(&std::fs::read(&earlier)?)?
    } else {
        Vec::new()
    };
    std::fs::write(&earlier, serde_json::to_vec_pretty(&items)?)?;
    let answered: BTreeSet<String> = items.iter().map(|item| item.id.clone()).collect();
    let mut ingested: Option<Ingested> = None;
    for question in sample.iter().filter(|question| !answered.contains(&question.question_id)) {
        eprintln!("{} {}", question.question_id, question.category());
        let shares_history =
            ingested.as_ref().is_some_and(|history| history.history_of.shares_history_with(question));
        let (ingestion_ms, exchanges_from) = if matches!(options.arm, Arm::Search) && !shares_history {
            drop(ingested.take());
            let history = ingest(question, &settings)?;
            let ingestion_ms = history.ingestion_ms;
            ingested = Some(history);
            (ingestion_ms, 0)
        } else {
            (0.0, ingested.as_ref().map_or(0, |history| history.daemon.exchange_count()))
        };
        items.push(evaluate(question, ingested.as_ref(), ingestion_ms, exchanges_from, &settings)?);
        std::fs::write(options.out.join("items.json"), serde_json::to_vec_pretty(&items)?)?;
    }
    report::write(&options.out)
}

pub fn shard_of(sample: Vec<BenchmarkItem>, shard: usize, shards: usize) -> Vec<BenchmarkItem> {
    let mut history = 0;
    let owners: Vec<usize> = (0..sample.len())
        .map(|index| {
            if index > 0 && !sample[index - 1].shares_history_with(&sample[index]) {
                history += 1;
            }
            history % shards
        })
        .collect();
    sample
        .into_iter()
        .zip(owners)
        .filter(|(_, owner)| *owner == shard)
        .map(|(question, _)| question)
        .collect()
}

fn ingest(question: &BenchmarkItem, settings: &Settings) -> anyhow::Result<Ingested> {
    let ingestion = Instant::now();
    let sign_in = settings.sign_in.as_deref();
    let daemon = Daemon::start(&settings.daemon_config, settings.models_dir.as_deref(), sign_in)?;
    let evidence = crate::ingest::replay(&daemon, question)?;
    daemon.finish()?;
    daemon.return_sign_in(sign_in)?;
    Ok(Ingested {
        history_of: question.clone(),
        daemon,
        evidence,
        ingestion_ms: ingestion.elapsed().as_secs_f64() * 1000.0,
    })
}

fn evaluate(
    question: &BenchmarkItem,
    ingested: Option<&Ingested>,
    ingestion_ms: f64,
    exchanges_from: usize,
    settings: &Settings,
) -> anyhow::Result<Item> {
    let daemon = ingested.map(|history| &history.daemon);
    let no_evidence = BTreeMap::new();
    let evidence = ingested.map_or(&no_evidence, |history| &history.evidence);
    let started = Instant::now();
    let results = if let Some(daemon) = daemon {
        let Response::SearchResults(results) = daemon.call(Request::Search(SearchQuery {
            text: question.question.clone(),
            k: Some(10),
            ..SearchQuery::default()
        }))?
        else {
            anyhow::bail!("search did not return results");
        };
        Some(results)
    } else {
        None
    };
    let answered = answer(question, results.as_deref(), started, settings)?;
    let (recall5, recall10, precision5) =
        crate::score::retrieval(results.as_deref(), evidence, question.evidence_sessions());
    Ok(Item {
        id: question.question_id.clone(),
        category: question.category().to_owned(),
        recall5,
        recall10,
        precision5,
        correct: answered.correct,
        latency_ms: answered.latency_ms,
        ingestion_ms,
        context_tokens: answered.context_tokens,
        answer: answered.answer,
        judge_answer: answered.judge_answer,
        results: serde_json::to_value(results)?,
        exchanges: daemon.map_or_else(Vec::new, |daemon| daemon.exchanges_since(exchanges_from)),
    })
}

pub struct Answered {
    pub answer: String,
    pub judge_answer: String,
    pub correct: bool,
    pub context_tokens: u64,
    pub latency_ms: f64,
}

pub fn answer(
    question: &BenchmarkItem,
    results: Option<&SearchResults>,
    started: Instant,
    settings: &Settings,
) -> anyhow::Result<Answered> {
    let context =
        results.map(|results| context::render(results, settings.context_budget)).unwrap_or_default();
    let (answer, context_tokens) = settings.reader.complete(
        include_str!("../prompts/reader.md"),
        &format!(
            "Memory facts:\n{context}\nQuestion date: {}\nQuestion: {}",
            question.question_date, question.question
        ),
    )?;
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    let prompts: Value = serde_json::from_str(include_str!("../prompts/judge.json"))?;
    let template = prompts[question.category()]
        .as_str()
        .or_else(|| prompts["default"].as_str())
        .context("judge prompt")?;
    let prompt = template
        .replace("{question}", &question.question)
        .replace("{answer}", &question.answer)
        .replace("{response}", &answer);
    let (judge_answer, _) =
        settings.answer_judge.complete("You are an impartial answer evaluator.", &prompt)?;
    let verdict = judge_answer.trim().trim_end_matches('.').to_lowercase();
    ensure!(verdict == "yes" || verdict == "no", "invalid judge verdict: {judge_answer}");
    Ok(Answered { answer, judge_answer, correct: verdict == "yes", context_tokens, latency_ms })
}
