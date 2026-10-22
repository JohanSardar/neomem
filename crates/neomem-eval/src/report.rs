use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub category: String,
    pub recall5: Option<f64>,
    pub recall10: Option<f64>,
    pub precision5: Option<f64>,
    pub correct: bool,
    #[serde(default)]
    pub failed_extractions: usize,
    pub latency_ms: f64,
    pub ingestion_ms: f64,
    pub context_tokens: u64,
    pub answer: String,
    pub judge_answer: String,
    pub results: Value,
    pub exchanges: Vec<Value>,
}

#[derive(Debug, Default, Serialize)]
struct Metrics {
    questions: u32,
    retrieval_questions: u32,
    questions_with_failed_extractions: u32,
    recall5: f64,
    recall10: f64,
    precision5: f64,
    accuracy: f64,
    latency_ms: f64,
    ingestion_ms: f64,
    context_tokens: f64,
}

pub fn write(directory: &Path) -> anyhow::Result<()> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(directory.join("manifest.json"))?)?;
    let items: Vec<Item> = serde_json::from_slice(&std::fs::read(directory.join("items.json"))?)?;
    let mut by_type = BTreeMap::<String, Metrics>::new();
    for item in &items {
        let metrics = by_type.entry(item.category.clone()).or_default();
        metrics.questions += 1;
        metrics.questions_with_failed_extractions += u32::from(item.failed_extractions > 0);
        if let Some(recall) = item.recall5 {
            metrics.retrieval_questions += 1;
            metrics.recall5 += recall;
            metrics.recall10 += item.recall10.unwrap_or_default();
            metrics.precision5 += item.precision5.unwrap_or_default();
        }
        metrics.accuracy += f64::from(item.correct);
        metrics.latency_ms += item.latency_ms;
        metrics.ingestion_ms += item.ingestion_ms;
        #[allow(clippy::cast_precision_loss)]
        {
            metrics.context_tokens += item.context_tokens as f64;
        }
    }
    for metrics in by_type.values_mut() {
        let n = f64::from(metrics.questions);
        metrics.accuracy /= n;
        metrics.latency_ms /= n;
        metrics.ingestion_ms /= n;
        metrics.context_tokens /= n;
        if metrics.retrieval_questions > 0 {
            let n = f64::from(metrics.retrieval_questions);
            metrics.recall5 /= n;
            metrics.recall10 /= n;
            metrics.precision5 /= n;
        }
    }
    let report = serde_json::json!({"settings":manifest,"by_type":by_type,"completed":items.len()});
    std::fs::write(directory.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
