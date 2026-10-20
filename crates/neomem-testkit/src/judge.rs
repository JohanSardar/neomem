use std::sync::{Mutex, PoisonError};

use neomem_core::{Choice, Decision, Judge, ModelError, ModelId};
use serde::{Deserialize, Serialize};

use crate::HashEmbedder;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeRule {
    #[serde(default)]
    pub question_contains: Option<String>,
    #[serde(default)]
    pub context_contains: Option<String>,
    #[serde(default)]
    pub candidate_contains: Option<String>,
    pub answer: JudgeAnswer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeAnswer {
    Choice { label: String, probability: f32 },
    Probability(f32),
}

#[derive(Debug, Default)]
pub struct ScriptedJudge {
    rules: Mutex<Vec<JudgeRule>>,
}

impl ScriptedJudge {
    pub fn with_rules(rules: Vec<JudgeRule>) -> Self {
        Self { rules: Mutex::new(rules) }
    }

    pub fn add_rules(&self, rules: Vec<JudgeRule>) {
        self.rules.lock().unwrap_or_else(PoisonError::into_inner).extend(rules);
    }

    fn matching(&self, question: &str, context: &str, candidate: Option<&str>) -> Option<JudgeAnswer> {
        let contains = |pattern: &Option<String>, text: &str| {
            pattern.as_ref().is_none_or(|needle| text.contains(needle))
        };
        let rules = self.rules.lock().unwrap_or_else(PoisonError::into_inner);
        rules
            .iter()
            .rev()
            .find(|rule| {
                contains(&rule.question_contains, question)
                    && contains(&rule.context_contains, context)
                    && (rule.candidate_contains.is_none()
                        || candidate.is_some_and(|text| contains(&rule.candidate_contains, text)))
            })
            .map(|rule| rule.answer.clone())
    }
}

impl Judge for ScriptedJudge {
    fn decide(&self, context: &str, question: &str, options: &[Choice<'_>]) -> Result<Decision, ModelError> {
        let preferred = match self.matching(question, context, None) {
            Some(JudgeAnswer::Choice { label, probability }) => Some((label, probability)),
            _ => options
                .iter()
                .find(|option| option.label == "new")
                .map(|option| (option.label.to_owned(), 0.9)),
        };
        let (label, probability) =
            preferred.unwrap_or_else(|| (options.first().map_or("", |option| option.label).to_owned(), 0.9));
        let alternatives = u16::try_from(options.len().saturating_sub(1).max(1)).map_or(1.0, f32::from);
        let others = (1.0 - probability) / alternatives;
        let probabilities =
            options.iter().map(|option| if option.label == label { probability } else { others }).collect();
        Ok(Decision { probabilities })
    }

    fn score(&self, context: &str, question: &str, candidates: &[&str]) -> Result<Vec<f32>, ModelError> {
        Ok(candidates
            .iter()
            .map(|candidate| match self.matching(question, context, Some(candidate)) {
                Some(JudgeAnswer::Probability(probability)) => probability,
                _ => HashEmbedder::vector(candidate).cosine(&HashEmbedder::vector(context)).clamp(0.0, 1.0),
            })
            .collect())
    }

    fn model_id(&self) -> ModelId {
        ModelId::new("fake:scripted-judge")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELATIONS: &[Choice<'static>] = &[
        Choice { label: "same", meaning: "says the same thing" },
        Choice { label: "updates", meaning: "replaces it" },
        Choice { label: "new", meaning: "unrelated" },
    ];

    #[test]
    fn without_a_rule_a_relation_question_answers_new() {
        let decision = ScriptedJudge::default().decide("ctx", "relation?", RELATIONS).unwrap();
        assert_eq!(RELATIONS[decision.choice()].label, "new");
    }

    #[test]
    fn the_newest_matching_rule_wins() {
        let judge = ScriptedJudge::with_rules(vec![JudgeRule {
            question_contains: None,
            context_contains: Some("ship date".into()),
            candidate_contains: None,
            answer: JudgeAnswer::Choice { label: "updates".into(), probability: 0.97 },
        }]);
        let decision = judge.decide("the ship date moved", "relation?", RELATIONS).unwrap();
        assert_eq!(RELATIONS[decision.choice()].label, "updates");
        assert!((decision.confidence() - 0.97).abs() < 1e-6);
    }
}
