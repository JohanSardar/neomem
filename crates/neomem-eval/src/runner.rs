use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, ensure};
use clap::{Args, ValueEnum};
use neomem_proto::{Request, Response, SearchQuery};
use serde::Serialize;
use serde_json::{Value, json};

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
}

pub fn run(options: &Options) -> anyhow::Result<()> {
    ensure!(!options.out.exists(), "output directory already exists");
    let settings: Settings = serde_json::from_slice(&std::fs::read(&options.models)?)?;
    let (questions, checksum) = data::load(&options.dataset)?;
    let (sample, held_out) = data::split(questions, options.sample, options.seed);
    ensure!(!sample.is_empty(), "empty sample");
    std::fs::create_dir_all(&options.out)?;
    let manifest = json!({"dataset_sha256":checksum,"seed":options.seed,"arm":options.arm,
        "selected":sample.iter().map(|q| &q.question_id).collect::<Vec<_>>(),"held_out":held_out,
        "models":settings,"k":10,"timezone":"UTC","reader_prompt":include_str!("../prompts/reader.md"),
        "judge_prompts":include_str!("../prompts/judge.json"),"git_revision":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().map(|o|String::from_utf8_lossy(&o.stdout).trim().to_owned())});
    std::fs::write(options.out.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    let mut items = Vec::new();
    for question in sample {
        eprintln!("{} {}", question.question_id, question.category());
        items.push(evaluate(&question, options.arm, &settings)?);
        std::fs::write(options.out.join("items.json"), serde_json::to_vec_pretty(&items)?)?;
    }
    report::write(&options.out)
}

fn evaluate(question: &BenchmarkItem, arm: Arm, settings: &Settings) -> anyhow::Result<Item> {
    let ingestion = Instant::now();
    let (daemon, evidence) = if matches!(arm, Arm::Search) {
        let daemon = Daemon::start(&settings.daemon_config, settings.models_dir.as_deref())?;
        let evidence = crate::ingest::replay(&daemon, question)?;
        daemon.finish()?;
        (Some(daemon), evidence)
    } else {
        (None, BTreeMap::new())
    };
    let ingestion_ms = ingestion.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let results = if let Some(daemon) = &daemon {
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
    let mut context = String::new();
    if let Some(results) = &results {
        for hit in &results.hits {
            let fact = format!("{}\n", hit.fact.content());
            if context.chars().count() + fact.chars().count() > settings.context_budget {
                break;
            }
            context.push_str(&fact);
        }
    }
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
    let (recall5, recall10, precision5) =
        crate::score::retrieval(results.as_deref(), &evidence, question.evidence_sessions());
    Ok(Item {
        id: question.question_id.clone(),
        category: question.category().to_owned(),
        recall5,
        recall10,
        precision5,
        correct: verdict == "yes",
        latency_ms,
        ingestion_ms,
        context_tokens,
        answer,
        judge_answer,
        results: serde_json::to_value(results)?,
        exchanges: daemon.as_ref().map_or_else(Vec::new, |d| {
            d.exchanges.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
        }),
    })
}
