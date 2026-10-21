use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use neomem_core::{Llm, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::extractor::{EXTRACTOR_MARKER, default_extraction};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmRule {
    #[serde(default)]
    pub system_contains: Option<String>,
    #[serde(default)]
    pub user_contains: Option<String>,
    pub reply: String,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default)]
    pub delta_delay_ms: u64,
    #[serde(default)]
    pub times: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScriptedAnswer {
    pub deltas: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub delay_ms: u64,
    pub delta_delay_ms: u64,
}

impl ScriptedAnswer {
    pub fn text(&self) -> String {
        self.deltas.concat()
    }

    pub fn reply(&self) -> LlmReply {
        LlmReply { text: self.text(), tool_calls: self.tool_calls.clone() }
    }

    fn of_text(text: &str) -> Self {
        Self { deltas: split_into_deltas(text), ..Self::default() }
    }
}

#[derive(Debug, Default)]
pub struct ScriptedLlm {
    rules: Mutex<Vec<LlmRule>>,
}

impl ScriptedLlm {
    pub fn with_rules(rules: Vec<LlmRule>) -> Self {
        Self { rules: Mutex::new(rules) }
    }

    pub fn add_rules(&self, rules: Vec<LlmRule>) {
        self.rules.lock().unwrap_or_else(PoisonError::into_inner).extend(rules);
    }

    pub fn take_rule(&self, system: &str, user: &str) -> Option<LlmRule> {
        let contains = |pattern: &Option<String>, text: &str| {
            pattern.as_ref().is_none_or(|needle| text.contains(needle))
        };
        let mut rules = self.rules.lock().unwrap_or_else(PoisonError::into_inner);
        let rule = rules.iter_mut().rev().find(|rule| {
            rule.times != Some(0)
                && contains(&rule.system_contains, system)
                && contains(&rule.user_contains, user)
        })?;
        if let Some(times) = rule.times.as_mut() {
            *times -= 1;
        }
        Some(rule.clone())
    }

    pub fn answer(&self, request: &LlmRequest) -> ScriptedAnswer {
        if let Some(LlmMessage::ToolResult { content, .. }) = request.messages.last() {
            return ScriptedAnswer::of_text(&format!("The tool answered: {content}"));
        }
        if let Some(rule) = self.take_rule(&request.system, &last_user_text(request)) {
            return ScriptedAnswer {
                deltas: split_into_deltas(&rule.reply),
                tool_calls: Vec::new(),
                delay_ms: rule.delay_ms,
                delta_delay_ms: rule.delta_delay_ms,
            };
        }
        if let Some(tool) = request.tools.first() {
            let schema = serde_json::from_str(&tool.parameters_schema).unwrap_or(Value::Null);
            let call = ToolCall {
                id: "call_fake_1".to_owned(),
                name: tool.name.clone(),
                arguments: example_for(&schema).to_string(),
            };
            return ScriptedAnswer { tool_calls: vec![call], ..ScriptedAnswer::default() };
        }
        if request.system.contains(EXTRACTOR_MARKER) {
            return ScriptedAnswer::of_text(&default_extraction(&last_user_text(request)).to_string());
        }
        if let Some(schema) = &request.response_schema {
            let schema = serde_json::from_str(schema).unwrap_or(Value::Null);
            return ScriptedAnswer::of_text(&example_for(&schema).to_string());
        }
        ScriptedAnswer::of_text("OK")
    }
}

pub fn last_user_text(request: &LlmRequest) -> String {
    request
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            LlmMessage::User(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn split_into_deltas(text: &str) -> Vec<String> {
    text.split_inclusive(char::is_whitespace).map(str::to_owned).collect()
}

pub fn example_for(schema: &Value) -> Value {
    if let Some(first) = schema["enum"].as_array().and_then(|options| options.first()) {
        return first.clone();
    }
    if let Some(first) = schema["anyOf"].as_array().and_then(|options| options.first()) {
        return example_for(first);
    }
    match schema["type"].as_str() {
        Some("object") => {
            let empty = serde_json::Map::new();
            let properties = schema["properties"].as_object().unwrap_or(&empty);
            Value::Object(properties.iter().map(|(name, inner)| (name.clone(), example_for(inner))).collect())
        }
        Some("array") => json!([example_for(&schema["items"])]),
        Some("integer" | "number") => json!(1),
        Some("boolean") => json!(true),
        Some("null") => Value::Null,
        _ => json!("ok"),
    }
}

impl Llm for ScriptedLlm {
    fn complete(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let answer = self.answer(request);
        thread::sleep(Duration::from_millis(answer.delay_ms));
        for (index, delta) in answer.deltas.iter().enumerate() {
            if index > 0 {
                thread::sleep(Duration::from_millis(answer.delta_delay_ms));
            }
            on_text(delta);
        }
        Ok(answer.reply())
    }

    fn model_id(&self) -> ModelId {
        ModelId::new("fake:scripted-llm")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neomem_core::ToolSpec;

    fn request(user: &str) -> LlmRequest {
        LlmRequest {
            system: "system".to_owned(),
            messages: vec![LlmMessage::User(user.to_owned())],
            response_schema: None,
            tools: Vec::new(),
            max_output_tokens: 0,
        }
    }

    #[test]
    fn a_rule_with_times_runs_out_and_the_next_rule_answers() {
        let llm = ScriptedLlm::with_rules(vec![
            LlmRule { reply: "good".into(), ..rule() },
            LlmRule { reply: "bad".into(), times: Some(1), ..rule() },
        ]);
        assert_eq!(llm.answer(&request("hi")).text(), "bad");
        assert_eq!(llm.answer(&request("hi")).text(), "good");
    }

    #[test]
    fn tools_are_called_with_arguments_shaped_like_their_schema() {
        let mut with_tool = request("weather?");
        with_tool.tools.push(ToolSpec {
            name: "lookup".into(),
            description: "looks up".into(),
            parameters_schema: r#"{"type":"object","properties":{"city":{"type":"string"}}}"#.into(),
        });
        let answer = ScriptedLlm::default().answer(&with_tool);
        assert_eq!(answer.tool_calls[0].arguments, r#"{"city":"ok"}"#);
    }

    #[test]
    fn replies_stream_word_by_word() {
        assert_eq!(split_into_deltas("one two\nthree"), ["one ", "two\n", "three"]);
    }

    fn rule() -> LlmRule {
        LlmRule {
            system_contains: None,
            user_contains: None,
            reply: String::new(),
            delay_ms: 0,
            delta_delay_ms: 0,
            times: None,
        }
    }
}
