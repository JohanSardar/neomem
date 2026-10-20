use std::time::Instant;

use neomem_core::{
    Choice, JudgeRole, LlmMessage, LlmRequest, LlmRole, ModelError, ModelRole, Purpose, Source,
};
use neomem_proto::{FailureKind, ModelTestReport, Response};

use crate::Engine;
use crate::reply::failed;

const YES_NO: &[Choice<'static>] = &[
    Choice { label: "yes", meaning: "the statement is true" },
    Choice { label: "no", meaning: "the statement is false" },
];

const STRUCTURED_TEST_SCHEMA: &str = r#"{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}"#;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModelProbe<'request> {
    pub role: ModelRole,
    pub source: Option<&'request Source>,
    pub structured: bool,
}

struct ProbeAnswer {
    model: String,
    answer: String,
    first_text_ms: Option<f64>,
}

impl ProbeAnswer {
    fn plain(model: String, answer: String) -> Self {
        Self { model, answer, first_text_ms: None }
    }
}

impl Engine {
    pub(crate) fn model_roster(&self) -> Response {
        Response::ModelRoster(self.models.describe())
    }

    pub(crate) fn test_model(&self, probe: ModelProbe<'_>) -> Response {
        let started = Instant::now();
        let outcome = match probe.role {
            ModelRole::Embedder => self.test_embedder(),
            ModelRole::Judge => self.test_judge(JudgeRole::Judge),
            ModelRole::Checker => self.test_judge(JudgeRole::Checker),
            ModelRole::Planner => self.test_llm(LlmRole::Planner, probe, started),
            ModelRole::Extractor => self.test_llm(LlmRole::Extractor, probe, started),
            ModelRole::Consolidator => self.test_llm(LlmRole::Consolidator, probe, started),
        };
        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        match outcome {
            Ok(answer) => Response::ModelTested(ModelTestReport {
                role: probe.role,
                model: answer.model,
                latency_ms,
                first_text_ms: answer.first_text_ms,
                answer: answer.answer,
            }),
            Err(error) => failed(FailureKind::Unavailable, format!("{}: {error}", probe.role.name())),
        }
    }

    pub(crate) fn start_login(&self, provider: String) -> Response {
        match self.models.start_login(&provider) {
            Ok(url) => Response::LoginOpened { provider, url },
            Err(error) => failed(FailureKind::Unavailable, format!("login {provider}: {error}")),
        }
    }

    pub(crate) fn finish_login(&self, provider: String) -> Response {
        match self.models.finish_login(&provider) {
            Ok(account) => Response::LoggedIn { provider, account },
            Err(error) => failed(FailureKind::Unavailable, format!("login {provider}: {error}")),
        }
    }

    fn test_embedder(&self) -> Result<ProbeAnswer, ModelError> {
        let embedder = self.models.embedder()?;
        let vectors = embedder.embed(&["neomem model test"], Purpose::Query)?;
        let dims = vectors.first().map_or(0, neomem_core::Vector::dims);
        Ok(ProbeAnswer::plain(embedder.model_id().to_string(), format!("{dims} dimensions")))
    }

    fn test_judge(&self, role: JudgeRole) -> Result<ProbeAnswer, ModelError> {
        let judge = self.models.judge(role)?;
        let decision = judge.decide("The user asks for a test.", "Is this a test request?", YES_NO)?;
        let label = YES_NO.get(decision.choice()).map_or("", |option| option.label);
        Ok(ProbeAnswer::plain(
            judge.model_id().to_string(),
            format!("{label} ({:.2})", decision.confidence()),
        ))
    }

    fn test_llm(
        &self,
        role: LlmRole,
        probe: ModelProbe<'_>,
        started: Instant,
    ) -> Result<ProbeAnswer, ModelError> {
        let llm = self.models.llm(role, probe.source)?;
        let request = if probe.structured { structured_probe() } else { plain_probe() };
        let mut first_text_ms = None;
        let reply = llm.complete(&request, &mut |delta| {
            if first_text_ms.is_none() && !delta.is_empty() {
                first_text_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
        })?;
        Ok(ProbeAnswer {
            model: llm.model_id().to_string(),
            answer: reply.text.trim().to_owned(),
            first_text_ms,
        })
    }
}

fn plain_probe() -> LlmRequest {
    LlmRequest {
        system: "Answer with the single word OK.".to_owned(),
        messages: vec![LlmMessage::User("ping".to_owned())],
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: 16,
    }
}

fn structured_probe() -> LlmRequest {
    LlmRequest {
        system: "Answer the message as JSON with one field, answer.".to_owned(),
        messages: vec![LlmMessage::User("neomem structured test: answer with the word ok".to_owned())],
        response_schema: Some(STRUCTURED_TEST_SCHEMA.to_owned()),
        tools: Vec::new(),
        max_output_tokens: 64,
    }
}
