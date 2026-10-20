use std::time::Instant;

use neomem_core::{Choice, JudgeRole, LlmMessage, LlmRequest, LlmRole, ModelError, ModelRole, Purpose};
use neomem_proto::{FailureKind, ModelTestReport, Response};

use crate::Engine;
use crate::reply::failed;

const YES_NO: &[Choice<'static>] = &[
    Choice { label: "yes", meaning: "the statement is true" },
    Choice { label: "no", meaning: "the statement is false" },
];

impl Engine {
    pub(crate) fn model_roster(&self) -> Response {
        Response::ModelRoster(self.models.describe())
    }

    pub(crate) fn test_model(&self, role: ModelRole) -> Response {
        let started = Instant::now();
        let outcome = match role {
            ModelRole::Embedder => self.test_embedder(),
            ModelRole::Judge => self.test_judge(JudgeRole::Judge),
            ModelRole::Checker => self.test_judge(JudgeRole::Checker),
            ModelRole::Planner => self.test_llm(LlmRole::Planner),
            ModelRole::Extractor => self.test_llm(LlmRole::Extractor),
            ModelRole::Consolidator => self.test_llm(LlmRole::Consolidator),
        };
        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            Ok((model, answer)) => Response::ModelTested(ModelTestReport { role, model, latency_ms, answer }),
            Err(error) => failed(FailureKind::Unavailable, format!("{}: {error}", role.name())),
        }
    }

    fn test_embedder(&self) -> Result<(String, String), ModelError> {
        let embedder = self.models.embedder()?;
        let vectors = embedder.embed(&["neomem model test"], Purpose::Query)?;
        let dims = vectors.first().map_or(0, neomem_core::Vector::dims);
        Ok((embedder.model_id().to_string(), format!("{dims} dimensions")))
    }

    fn test_judge(&self, role: JudgeRole) -> Result<(String, String), ModelError> {
        let judge = self.models.judge(role)?;
        let decision = judge.decide("The user asks for a test.", "Is this a test request?", YES_NO)?;
        let label = YES_NO.get(decision.choice()).map_or("", |option| option.label);
        Ok((judge.model_id().to_string(), format!("{label} ({:.2})", decision.confidence())))
    }

    fn test_llm(&self, role: LlmRole) -> Result<(String, String), ModelError> {
        let llm = self.models.llm(role, None)?;
        let request = LlmRequest {
            system: "Answer with the single word OK.".to_owned(),
            messages: vec![LlmMessage::User("ping".to_owned())],
            response_schema: None,
            tools: Vec::new(),
            max_output_tokens: 16,
        };
        let reply = llm.complete(&request, &mut |_| {})?;
        Ok((llm.model_id().to_string(), reply.text.trim().to_owned()))
    }
}
