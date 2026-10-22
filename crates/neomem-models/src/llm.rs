use std::fmt::Debug;
use std::time::Duration;

use neomem_core::{Effort, Llm, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolSpec};
use serde_json::Value;

use crate::json_schema;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

const JSON_ONLY: &str = include_str!("../prompts/json_only.md");
const JSON_RETRY: &str = include_str!("../prompts/json_retry.md");

#[derive(Debug, Clone)]
pub struct LlmSettings {
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub fast: bool,
    pub timeout: Duration,
}

impl LlmSettings {
    pub fn required_model(&self, provider: &str) -> Result<String, ModelError> {
        self.model
            .clone()
            .ok_or_else(|| ModelError::NotConfigured(format!("{provider}: the role names no model")))
    }
}

pub trait Wire: Send + Sync + Debug {
    fn has_schema_mode(&self) -> bool;
    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError>;
    fn model_id(&self) -> ModelId;
}

#[derive(Debug)]
pub struct Provider<W>(pub W);

impl<W: Wire> Llm for Provider<W> {
    fn complete(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        match &request.response_schema {
            None => self.0.send(request, on_text),
            Some(schema) => complete_structured(&self.0, request, schema, on_text),
        }
    }

    fn model_id(&self) -> ModelId {
        self.0.model_id()
    }
}

fn complete_structured(
    wire: &dyn Wire,
    request: &LlmRequest,
    schema_text: &str,
    on_text: &mut dyn FnMut(&str),
) -> Result<LlmReply, ModelError> {
    let schema = parse_schema(schema_text)?;
    let mut asked = request.clone();
    if !wire.has_schema_mode() {
        asked.system = with_json_only_instructions(&request.system, schema_text);
    }
    let (rejected, problem) = match fitted(wire.send(&asked, &mut |_| {})?, &schema) {
        Ok(reply) => return Ok(delivered(reply, on_text)),
        Err(rejection) => rejection,
    };
    asked.messages.push(LlmMessage::Assistant { text: rejected, tool_calls: Vec::new() });
    asked.messages.push(LlmMessage::User(JSON_RETRY.trim().replace("{problem}", &problem)));
    match fitted(wire.send(&asked, &mut |_| {})?, &schema) {
        Ok(reply) => Ok(delivered(reply, on_text)),
        Err((_, problem)) => {
            Err(ModelError::InvalidOutput(format!("the answer broke the schema twice; last time {problem}")))
        }
    }
}

fn delivered(reply: LlmReply, on_text: &mut dyn FnMut(&str)) -> LlmReply {
    if !reply.text.is_empty() {
        on_text(&reply.text);
    }
    reply
}

fn with_json_only_instructions(system: &str, schema: &str) -> String {
    let instructions = JSON_ONLY.trim().replace("{schema}", schema);
    if system.trim().is_empty() { instructions } else { format!("{}\n\n{instructions}", system.trim_end()) }
}

fn fitted(reply: LlmReply, schema: &Value) -> Result<LlmReply, (String, String)> {
    if !reply.tool_calls.is_empty() {
        return Ok(reply);
    }
    let candidate = without_code_fence(&reply.text);
    match serde_json::from_str::<Value>(candidate) {
        Err(error) => Err((reply.text.clone(), format!("it is not JSON ({error})"))),
        Ok(value) => match json_schema::violation(&value, schema) {
            Some(problem) => Err((reply.text.clone(), problem)),
            None => Ok(LlmReply { text: candidate.to_owned(), tool_calls: Vec::new() }),
        },
    }
}

fn without_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(opened) = trimmed.strip_prefix("```") else { return trimmed };
    let body = opened.split_once('\n').map_or("", |(_, rest)| rest);
    body.strip_suffix("```").unwrap_or(body).trim()
}

pub fn parse_schema(schema: &str) -> Result<Value, ModelError> {
    serde_json::from_str(schema)
        .map_err(|error| ModelError::InvalidOutput(format!("the response schema is not JSON: {error}")))
}

pub fn tool_parameters(tool: &ToolSpec) -> Result<Value, ModelError> {
    serde_json::from_str(&tool.parameters_schema).map_err(|error| {
        ModelError::InvalidOutput(format!("the parameters schema of tool {} is not JSON: {error}", tool.name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct Replay {
        schema_mode: bool,
        answers: Mutex<Vec<&'static str>>,
        seen: Mutex<Vec<LlmRequest>>,
    }

    impl Wire for Replay {
        fn has_schema_mode(&self) -> bool {
            self.schema_mode
        }

        fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
            self.seen.lock().unwrap().push(request.clone());
            let text = self.answers.lock().unwrap().remove(0);
            on_text(text);
            Ok(LlmReply { text: text.to_owned(), tool_calls: Vec::new() })
        }

        fn model_id(&self) -> ModelId {
            ModelId::new("replay")
        }
    }

    fn structured_request() -> LlmRequest {
        LlmRequest {
            system: "Plan queries.".to_owned(),
            messages: vec![LlmMessage::User("signing error again".to_owned())],
            response_schema: Some(
                r#"{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}"#.into(),
            ),
            tools: Vec::new(),
            max_output_tokens: 0,
        }
    }

    fn replay(schema_mode: bool, answers: Vec<&'static str>) -> Provider<Replay> {
        Provider(Replay { schema_mode, answers: Mutex::new(answers), seen: Mutex::new(Vec::new()) })
    }

    #[test]
    fn without_a_schema_mode_the_prompt_asks_for_json_and_a_bad_answer_is_retried_once() {
        let provider = replay(false, vec!["Sure, here it is", "```json\n{\"q\":\"edge\"}\n```"]);
        let mut streamed = String::new();
        let reply = provider.complete(&structured_request(), &mut |text| streamed.push_str(text)).unwrap();
        assert_eq!(reply.text, r#"{"q":"edge"}"#);
        assert_eq!(streamed, reply.text);
        let seen = provider.0.seen.lock().unwrap();
        assert!(seen[0].system.contains("JSON Schema"));
        assert_eq!(seen.len(), 2);
        assert!(
            matches!(&seen[1].messages[1], LlmMessage::Assistant { text, .. } if text == "Sure, here it is")
        );
    }

    #[test]
    fn two_bad_answers_fail_with_invalid_output() {
        let provider = replay(true, vec!["{}", "{\"q\": 3}"]);
        let outcome = provider.complete(&structured_request(), &mut |_| {});
        assert!(matches!(outcome, Err(ModelError::InvalidOutput(_))));
        assert!(!provider.0.seen.lock().unwrap()[0].system.contains("JSON Schema"));
    }
}
