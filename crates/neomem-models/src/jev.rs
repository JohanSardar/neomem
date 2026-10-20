use std::collections::BTreeMap;
use std::time::Duration;

use neomem_core::{Choice, Decision, Judge, ModelError, ModelId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http;

pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(600);

#[derive(Debug)]
pub struct JevJudge {
    agent: ureq::Agent,
    url: String,
    key: String,
    model: String,
    timeout: Duration,
}

#[derive(Serialize)]
struct SystemOneRequest<'call> {
    model: &'call str,
    state: &'call str,
    questions: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct SystemOneResponse {
    answers: BTreeMap<String, Answer>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Answer {
    Noul { noul: f32 },
    Choice { probabilities: BTreeMap<String, f32> },
    Score {},
}

impl JevJudge {
    pub fn new(base_url: Option<&str>, key: String, model: &str, timeout: Duration) -> Self {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/');
        Self {
            agent: http::agent(timeout),
            url: format!("{base}/v1/systemone"),
            key,
            model: model.to_owned(),
            timeout,
        }
    }

    fn ask(
        &self,
        state: &str,
        questions: BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Answer>, ModelError> {
        let body = SystemOneRequest { model: &self.model, state, questions };
        let response: SystemOneResponse =
            http::post_json(&self.agent, &self.url, Some(&self.key), &body, self.timeout)?;
        Ok(response.answers)
    }
}

impl Judge for JevJudge {
    fn decide(&self, context: &str, question: &str, options: &[Choice<'_>]) -> Result<Decision, ModelError> {
        if options.is_empty() {
            return Ok(Decision { probabilities: Vec::new() });
        }
        let criteria: serde_json::Map<String, Value> =
            options.iter().map(|option| (option.label.to_owned(), Value::from(option.meaning))).collect();
        let questions = BTreeMap::from([(
            "decision".to_owned(),
            serde_json::json!({ "type": "choice", "instructions": question, "criteria": criteria }),
        )]);
        let mut answers = self.ask(context, questions)?;
        let Some(Answer::Choice { probabilities }) = answers.remove("decision") else {
            return Err(ModelError::InvalidOutput("Jev returned no choice answer".to_owned()));
        };
        let ordered =
            options.iter().map(|option| probabilities.get(option.label).copied().unwrap_or(0.0)).collect();
        Ok(Decision { probabilities: ordered })
    }

    fn score(&self, context: &str, question: &str, candidates: &[&str]) -> Result<Vec<f32>, ModelError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let questions: BTreeMap<String, Value> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let instructions = format!("{question}\nCandidate: {candidate}");
                (format!("c{index:03}"), serde_json::json!({ "type": "noul", "instructions": instructions }))
            })
            .collect();
        let answers = self.ask(context, questions)?;
        (0..candidates.len())
            .map(|index| match answers.get(&format!("c{index:03}")) {
                Some(Answer::Noul { noul }) => Ok(*noul),
                _ => Err(ModelError::InvalidOutput(format!("Jev returned no answer for candidate {index}"))),
            })
            .collect()
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("jev:{}", self.model))
    }
}
