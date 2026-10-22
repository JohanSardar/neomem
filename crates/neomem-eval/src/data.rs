use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub haystack_sessions: Arc<Vec<Arc<Vec<Turn>>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub haystack_session_ids: Arc<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub haystack_dates: Arc<Vec<String>>,
    #[serde(skip)]
    pub history_id: Option<Arc<String>>,
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
    pub fn shares_history_with(&self, other: &Self) -> bool {
        if let (Some(first), Some(second)) = (&self.history_id, &other.history_id) {
            return first == second;
        }
        self.haystack_session_ids == other.haystack_session_ids
            && self.haystack_dates == other.haystack_dates
            && self.haystack_sessions == other.haystack_sessions
    }

    pub fn evidence_sessions(&self) -> &[String] {
        if self.category() == "abstention" { &[] } else { &self.answer_session_ids }
    }

    pub fn category(&self) -> &str {
        if self.question_id.ends_with("_abs") { "abstention" } else { &self.question_type }
    }
}

pub fn load(path: &Path) -> anyhow::Result<(Vec<BenchmarkItem>, String)> {
    let (questions, checksum, _) = load_with_split(path)?;
    Ok((questions, checksum))
}

pub fn load_with_split(
    path: &Path,
) -> anyhow::Result<(Vec<BenchmarkItem>, String, Option<crate::prepare::Split>)> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 65536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let mut probe = BufReader::new(std::fs::File::open(path)?);
    let first = probe
        .by_ref()
        .bytes()
        .find_map(|byte| match byte {
            Ok(byte) if byte.is_ascii_whitespace() => None,
            result => Some(result),
        })
        .context("empty dataset")??;
    let reader = BufReader::new(std::fs::File::open(path)?);
    let (questions, prepared): (Vec<BenchmarkItem>, _) = if first == b'[' {
        (serde_json::from_reader(reader)?, None)
    } else {
        let dataset: SharedDataset = serde_json::from_reader(reader)?;
        let prepared = dataset.prepared.clone();
        (dataset.expand()?, prepared)
    };
    for question in &questions {
        ensure!(
            question.haystack_sessions.len() == question.haystack_session_ids.len()
                && question.haystack_sessions.len() == question.haystack_dates.len(),
            "unaligned sessions: {}",
            question.question_id
        );
    }
    let mut checksum = String::new();
    for byte in digest.finalize() {
        write!(&mut checksum, "{byte:02x}")?;
    }
    Ok((questions, checksum, prepared))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct History {
    pub session_ids: Arc<Vec<String>>,
    pub dates: Arc<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_keys: Option<Arc<Vec<String>>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SharedQuestion {
    #[serde(flatten)]
    pub item: BenchmarkItem,
    pub history: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SharedDataset {
    pub format: String,
    pub sessions: BTreeMap<String, Arc<Vec<Turn>>>,
    pub histories: BTreeMap<String, History>,
    pub questions: Vec<SharedQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared: Option<crate::prepare::Split>,
}

impl SharedDataset {
    fn expand(self) -> anyhow::Result<Vec<BenchmarkItem>> {
        ensure!(self.format == "neomem-eval/2", "unsupported dataset format");
        let histories = self
            .histories
            .into_iter()
            .map(|(id, history)| {
                ensure!(history.session_ids.len() == history.dates.len(), "unaligned history: {id}");
                let keys = history.session_keys.as_ref().unwrap_or(&history.session_ids);
                ensure!(keys.len() == history.session_ids.len(), "unaligned session keys: {id}");
                let sessions = keys
                    .iter()
                    .map(|session| {
                        self.sessions
                            .get(session)
                            .cloned()
                            .with_context(|| format!("missing session: {session}"))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok((id.clone(), (Arc::new(id), history, Arc::new(sessions))))
            })
            .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
        self.questions
            .into_iter()
            .map(|question| {
                let (id, history, sessions) = histories.get(&question.history).context("missing history")?;
                let mut item = question.item;
                item.history_id = Some(id.clone());
                item.haystack_sessions = sessions.clone();
                item.haystack_session_ids = history.session_ids.clone();
                item.haystack_dates = history.dates.clone();
                Ok(item)
            })
            .collect()
    }
}

pub fn split(
    questions: Vec<BenchmarkItem>,
    sample: usize,
    seed: u64,
) -> (Vec<BenchmarkItem>, Vec<BenchmarkItem>) {
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
                held_out.push(question);
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
                    held_out.push(question.clone());
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
    fn shared_schema_loads_without_legacy_fields_and_validates_references() {
        let question = serde_json::json!({
            "question_id":"q1", "question_type":"single-hop", "question":"what?",
            "answer":3, "question_date":"2024/01/01", "answer_session_ids":["s1"], "history":"h1"
        });
        let mut second = question.clone();
        second["question_id"] = serde_json::json!("q2");
        second["history"] = serde_json::json!("h2");
        let mut dataset = serde_json::json!({
            "format":"neomem-eval/2", "sessions":{"s1":[{"role":"user", "content":"three"}]},
            "histories":{"h1":{"session_ids":["s1"], "dates":["2024/01/01"]},
                         "h2":{"session_ids":["s1"], "dates":["2024/01/01"]}},
            "questions":[question, second]
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.json");
        std::fs::write(&path, serde_json::to_vec(&dataset).unwrap()).unwrap();
        let (items, _) = load(&path).unwrap();
        assert_eq!(items[0].answer, "3");
        let (again, _) = load(&path).unwrap();
        assert!(items[0].shares_history_with(&again[0]));
        assert!(!items[0].shares_history_with(&items[1]));
        assert!(Arc::ptr_eq(&items[0].haystack_sessions[0], &items[1].haystack_sessions[0]));
        dataset["histories"]["h1"]["session_ids"] = serde_json::json!(["missing"]);
        std::fs::write(&path, serde_json::to_vec(&dataset).unwrap()).unwrap();
        assert!(load(&path).is_err());
        dataset["format"] = serde_json::json!("neomem-eval/3");
        std::fs::write(&path, serde_json::to_vec(&dataset).unwrap()).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn legacy_checksum_is_the_original_byte_checksum() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agents/skills/neomem-control/fixtures/eval/questions.json");
        let (_, checksum) = load(&path).unwrap();
        assert_eq!(checksum, "6afeff8915425b228268a9eeb33af41c8f24f392fa9423162c41872147af549a");
    }

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
        assert!(ids(&selected).iter().all(|id| !ids(&held_out).contains(id)));
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
