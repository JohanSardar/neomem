use std::fmt::{self, Debug};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::Source;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Vector(Vec<f32>);

impl Vector {
    pub fn normalized(mut values: Vec<f32>) -> Self {
        let length = values.iter().map(|value| value * value).sum::<f32>().sqrt();
        if length > f32::EPSILON {
            for value in &mut values {
                *value /= length;
            }
        }
        Self(values)
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    pub fn into_inner(self) -> Vec<f32> {
        self.0
    }

    pub fn dims(&self) -> usize {
        self.0.len()
    }

    pub fn cosine(&self, other: &Self) -> f32 {
        self.0.iter().zip(&other.0).map(|(left, right)| left * right).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("no model is configured for {0}")]
    NotConfigured(String),
    #[error("the model is unavailable: {0}")]
    Unavailable(String),
    #[error("the model did not answer within {0} ms")]
    Timeout(u64),
    #[error("the model answered with output that does not fit: {0}")]
    InvalidOutput(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    Query,
    Document,
}

pub trait Embedder: Send + Sync + Debug {
    fn embed(&self, texts: &[&str], purpose: Purpose) -> Result<Vec<Vector>, ModelError>;
    fn model_id(&self) -> ModelId;
    fn dims(&self) -> usize;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub probabilities: Vec<f32>,
}

impl Decision {
    pub fn choice(&self) -> usize {
        self.probabilities
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map_or(0, |(index, _)| index)
    }

    pub fn confidence(&self) -> f32 {
        self.probabilities.get(self.choice()).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice<'text> {
    pub label: &'text str,
    pub meaning: &'text str,
}

pub trait Judge: Send + Sync + Debug {
    fn decide(&self, context: &str, question: &str, options: &[Choice<'_>]) -> Result<Decision, ModelError>;
    fn score(&self, context: &str, question: &str, candidates: &[&str]) -> Result<Vec<f32>, ModelError>;
    fn model_id(&self) -> ModelId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmMessage {
    User(String),
    Assistant { text: String, tool_calls: Vec<ToolCall> },
    ToolResult { call_id: String, content: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmRequest {
    pub system: String,
    pub messages: Vec<LlmMessage>,
    pub response_schema: Option<String>,
    pub tools: Vec<ToolSpec>,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LlmReply {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

pub trait Llm: Send + Sync + Debug {
    fn complete(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError>;
    fn model_id(&self) -> ModelId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JudgeRole {
    Judge,
    Checker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmRole {
    Planner,
    Extractor,
    Consolidator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Embedder,
    Judge,
    Checker,
    Planner,
    Extractor,
    Consolidator,
}

impl ModelRole {
    pub const ALL: [Self; 6] =
        [Self::Embedder, Self::Judge, Self::Checker, Self::Planner, Self::Extractor, Self::Consolidator];

    pub fn name(self) -> &'static str {
        match self {
            Self::Embedder => "embedder",
            Self::Judge => "judge",
            Self::Checker => "checker",
            Self::Planner => "planner",
            Self::Extractor => "extractor",
            Self::Consolidator => "consolidator",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub role: ModelRole,
    pub assignment: String,
}

pub trait ModelRoles: Send + Sync + Debug {
    fn embedder(&self) -> Result<Arc<dyn Embedder>, ModelError>;
    fn judge(&self, role: JudgeRole) -> Result<Arc<dyn Judge>, ModelError>;
    fn llm(&self, role: LlmRole, source: Option<&Source>) -> Result<Arc<dyn Llm>, ModelError>;
    fn describe(&self) -> Vec<RoleAssignment>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_unit_length_so_cosine_is_a_dot_product() {
        let vector = Vector::normalized(vec![3.0, 4.0]);
        assert!((vector.cosine(&vector) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_decision_picks_the_most_probable_option() {
        let decision = Decision { probabilities: vec![0.1, 0.7, 0.2] };
        assert_eq!(decision.choice(), 1);
        assert!((decision.confidence() - 0.7).abs() < f32::EPSILON);
    }
}
