use std::sync::{Mutex, PoisonError};

use neomem_core::{Llm, LlmMessage, LlmReply, LlmRequest, ModelError, ModelId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmRule {
    #[serde(default)]
    pub system_contains: Option<String>,
    #[serde(default)]
    pub user_contains: Option<String>,
    pub reply: String,
    #[serde(default)]
    pub delay_ms: u64,
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

    pub fn rule_for(&self, system: &str, user: &str) -> Option<LlmRule> {
        let contains = |pattern: &Option<String>, text: &str| {
            pattern.as_ref().is_none_or(|needle| text.contains(needle))
        };
        let rules = self.rules.lock().unwrap_or_else(PoisonError::into_inner);
        rules
            .iter()
            .rev()
            .find(|rule| contains(&rule.system_contains, system) && contains(&rule.user_contains, user))
            .cloned()
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

impl Llm for ScriptedLlm {
    fn complete(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let user = last_user_text(request);
        let rule = self.rule_for(&request.system, &user).ok_or_else(|| {
            ModelError::Unavailable("the scripted LLM has no rule for this request".to_owned())
        })?;
        std::thread::sleep(std::time::Duration::from_millis(rule.delay_ms));
        on_text(&rule.reply);
        Ok(LlmReply { text: rule.reply, tool_calls: Vec::new() })
    }

    fn model_id(&self) -> ModelId {
        ModelId::new("fake:scripted-llm")
    }
}
