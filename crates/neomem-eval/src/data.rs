use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub has_answer: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Capture {
    pub agent: String,
    pub session: String,
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BenchmarkItem {
    #[serde(default)]
    pub captures: Vec<Capture>,
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    #[serde(deserialize_with = "answer_text")]
    pub answer: String,
    pub question_date: String,
    pub haystack_sessions: Vec<Vec<Turn>>,
    pub haystack_session_ids: Vec<String>,
    pub haystack_dates: Vec<String>,
    pub answer_session_ids: Vec<String>,
}

fn answer_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let answer = serde_json::Value::deserialize(deserializer)?;
    match answer {
        serde_json::Value::String(text) => Ok(text),
        serde_json::Value::Number(number) => Ok(number.to_string()),
        _ => Err(serde::de::Error::custom("gold answer must be text or a number")),
    }
}

impl BenchmarkItem {
    pub fn evidence_sessions(&self) -> &[String] {
        if self.category() == "abstention" { &[] } else { &self.answer_session_ids }
    }

    pub fn category(&self) -> &str {
        if self.question_id.ends_with("_abs") { "abstention" } else { &self.question_type }
    }
}

pub fn load(path: &Path) -> anyhow::Result<(Vec<BenchmarkItem>, String)> {
    let bytes = std::fs::read(path)?;
    let questions: Vec<BenchmarkItem> = serde_json::from_slice(&bytes)?;
    for question in &questions {
        ensure!(
            question.haystack_sessions.len() == question.haystack_session_ids.len()
                && question.haystack_sessions.len() == question.haystack_dates.len(),
            "unaligned sessions: {}",
            question.question_id
        );
    }
    let mut checksum = String::new();
    for byte in Sha256::digest(&bytes) {
        write!(&mut checksum, "{byte:02x}")?;
    }
    Ok((questions, checksum))
}

pub fn split(questions: Vec<BenchmarkItem>, sample: usize, seed: u64) -> (Vec<BenchmarkItem>, Vec<String>) {
    let reserve = sample < questions.len();
    let mut groups = BTreeMap::<String, Vec<BenchmarkItem>>::new();
    for question in questions {
        groups.entry(question.category().to_owned()).or_default().push(question);
    }
    for group in groups.values_mut() {
        group.sort_by_cached_key(|q| Sha256::digest(format!("{seed}:{}", q.question_id)).to_vec());
    }
    let mut selected = Vec::new();
    let mut held_out = Vec::new();
    if reserve {
        for group in groups.values_mut() {
            if group.len() > 1
                && let Some(question) = group.pop()
            {
                held_out.push(question.question_id);
            }
        }
    }
    let mut round = 0;
    loop {
        let mut found = false;
        for group in groups.values() {
            if let Some(question) = group.get(round) {
                found = true;
                if selected.len() < sample {
                    selected.push(question.clone());
                } else {
                    held_out.push(question.question_id.clone());
                }
            }
        }
        if !found {
            break;
        }
        round += 1;
    }
    (selected, held_out)
}

pub fn date(text: &str) -> anyhow::Result<i64> {
    let normalized = text.replace('/', "-");
    let day = normalized.get(..10).context("missing session date")?;
    let time = normalized.split_whitespace().last().filter(|s| s.contains(':')).unwrap_or("00:00");
    let civil: jiff::civil::DateTime = format!("{day}T{time}:00").parse()?;
    Ok(civil.to_zoned(jiff::tz::TimeZone::UTC)?.timestamp().as_millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stratification_is_deterministic_disjoint_and_covers_types() {
        let fixture = include_str!("../../../.agents/skills/neomem-control/fixtures/eval/questions.json");
        let questions: Vec<BenchmarkItem> = serde_json::from_str(fixture).unwrap();
        let (selected, held_out) = split(questions.clone(), 3, 7);
        let (again, _) = split(questions, 3, 7);
        let ids = |items: &[BenchmarkItem]| items.iter().map(|q| q.question_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&selected), ids(&again));
        assert_eq!(selected.len(), 3);
        assert_eq!(held_out.len(), 1);
        assert!(selected.iter().all(|q| !held_out.contains(&q.question_id)));
        assert_eq!(
            selected.iter().map(BenchmarkItem::category).collect::<std::collections::BTreeSet<_>>().len(),
            3
        );
    }

    #[test]
    fn cleaned_benchmark_accepts_numeric_gold_answers() {
        let mut fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../.agents/skills/neomem-control/fixtures/eval/questions.json"
        ))
        .unwrap();
        fixture[0]["answer"] = serde_json::json!(3);
        let questions: Vec<BenchmarkItem> = serde_json::from_value(fixture).unwrap();
        assert_eq!(questions[0].answer, "3");
    }

    #[test]
    fn abstentions_ignore_retained_positive_evidence_ids() {
        let fixture = include_str!("../../../.agents/skills/neomem-control/fixtures/eval/questions.json");
        let questions: Vec<BenchmarkItem> = serde_json::from_str(fixture).unwrap();
        let abstention = questions.iter().find(|q| q.category() == "abstention").unwrap();
        assert_ne!(abstention.answer_session_ids, [] as [String; 0]);
        assert_eq!(abstention.evidence_sessions(), [] as [String; 0]);
    }

    #[test]
    fn dates_keep_session_time_in_utc() {
        assert_eq!(date("1970/01/02 (Fri) 01:30").unwrap(), 91_800_000);
        assert!(date("bad date").is_err());
    }
}
