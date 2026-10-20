use std::sync::Arc;

use neomem_core::{Choice, Decision, Judge, ModelError, ModelId};

#[derive(Debug)]
pub struct FallbackJudge {
    primary: Result<Arc<dyn Judge>, ModelError>,
    fallback: Arc<dyn Judge>,
}

impl FallbackJudge {
    pub fn new(primary: Result<Arc<dyn Judge>, ModelError>, fallback: Arc<dyn Judge>) -> Self {
        Self { primary, fallback }
    }

    fn answer<Answer>(
        &self,
        ask: impl Fn(&dyn Judge) -> Result<Answer, ModelError>,
    ) -> Result<Answer, ModelError> {
        let primary_problem = match &self.primary {
            Ok(primary) => match ask(primary.as_ref()) {
                Ok(answer) => return Ok(answer),
                Err(problem) => {
                    tracing::warn!(
                        primary = %primary.model_id(),
                        fallback = %self.fallback.model_id(),
                        "judge: {problem}; the local fallback answers this call"
                    );
                    problem
                }
            },
            Err(missing) => missing.clone(),
        };
        ask(self.fallback.as_ref()).map_err(|problem| {
            ModelError::Unavailable(format!(
                "{primary_problem}, and the fallback {} failed too: {problem}",
                self.fallback.model_id()
            ))
        })
    }
}

impl Judge for FallbackJudge {
    fn decide(&self, context: &str, question: &str, options: &[Choice<'_>]) -> Result<Decision, ModelError> {
        self.answer(|judge| judge.decide(context, question, options))
    }

    fn score(&self, context: &str, question: &str, candidates: &[&str]) -> Result<Vec<f32>, ModelError> {
        self.answer(|judge| judge.score(context, question, candidates))
    }

    fn model_id(&self) -> ModelId {
        self.primary.as_ref().map_or_else(|_| self.fallback.model_id(), |primary| primary.model_id())
    }
}

#[cfg(test)]
mod tests {
    use neomem_testkit::{JudgeAnswer, JudgeRule, ScriptedJudge, judge_contract};

    use super::*;

    #[derive(Debug)]
    struct Down;

    impl Judge for Down {
        fn decide(&self, _: &str, _: &str, _: &[Choice<'_>]) -> Result<Decision, ModelError> {
            Err(ModelError::Timeout(600))
        }

        fn score(&self, _: &str, _: &str, _: &[&str]) -> Result<Vec<f32>, ModelError> {
            Err(ModelError::Unavailable("connection refused".to_owned()))
        }

        fn model_id(&self) -> ModelId {
            ModelId::new("jev:down")
        }
    }

    fn sure(probability: f32) -> Arc<dyn Judge> {
        Arc::new(ScriptedJudge::with_rules(vec![JudgeRule {
            question_contains: None,
            context_contains: None,
            candidate_contains: None,
            answer: JudgeAnswer::Probability(probability),
        }]))
    }

    #[test]
    fn a_failing_primary_hands_each_call_to_the_fallback() {
        let judge = FallbackJudge::new(Ok(Arc::new(Down)), sure(0.8));
        judge_contract(&judge);
        assert_eq!(judge.score("context", "question", &["one"]).unwrap(), vec![0.8]);
        assert_eq!(judge.model_id().as_str(), "jev:down");
    }

    #[test]
    fn a_working_primary_answers_and_a_missing_one_names_the_fallback() {
        let working = FallbackJudge::new(Ok(sure(0.3)), sure(0.8));
        assert_eq!(working.score("context", "question", &["one"]).unwrap(), vec![0.3]);
        let keyless = FallbackJudge::new(Err(ModelError::Unavailable("no key".to_owned())), sure(0.8));
        assert_eq!(keyless.score("context", "question", &["one"]).unwrap(), vec![0.8]);
        assert_eq!(keyless.model_id().as_str(), "fake:scripted-judge");
    }

    #[test]
    fn when_both_fail_the_error_names_both() {
        let judge = FallbackJudge::new(Ok(Arc::new(Down)), Arc::new(Down));
        let problem = judge.score("context", "question", &["one"]).unwrap_err().to_string();
        assert!(problem.contains("connection refused") && problem.contains("fallback"));
    }
}
