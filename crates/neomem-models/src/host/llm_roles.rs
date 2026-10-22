use std::sync::Arc;

use neomem_core::{Llm, LlmRole, ModelError, Source};

use super::ModelHost;
use crate::anthropic::{self, AnthropicLlm};
use crate::chatgpt::{self, ChatgptLlm};
use crate::codex::CodexLlm;
use crate::command::CommandLlm;
use crate::config::{ProviderKind, RoleConfig};
use crate::duration;
use crate::llm::{self, LlmSettings, Provider};
use crate::openai_chat::OpenAiChat;
use crate::openai_responses::OpenAiResponses;

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

impl ModelHost {
    pub(super) fn llm_role(
        &self,
        role: LlmRole,
        source: Option<&Source>,
    ) -> Result<(Option<String>, &RoleConfig), ModelError> {
        let roles = &self.config.models;
        if role == LlmRole::Extractor
            && let Some(source) = source
            && let Some(by_source) = roles.extractor_by_source.get(source.as_str())
        {
            return Ok((Some(source.as_str().to_owned()), by_source));
        }
        let configured = match role {
            LlmRole::Planner => roles.planner.as_ref(),
            LlmRole::Extractor => roles.extractor.as_ref(),
            LlmRole::Consolidator => roles.consolidator.as_ref(),
        };
        configured.map(|config| (None, config)).ok_or_else(|| ModelError::NotConfigured(format!("{role:?}")))
    }

    pub(super) fn build_llm(&self, role_name: &str, role: &RoleConfig) -> Result<Arc<dyn Llm>, ModelError> {
        let provider = self.provider(role, role_name)?;
        let settings = LlmSettings {
            model: role.model.clone().or_else(|| provider.model.clone()),
            effort: role.effort,
            fast: role.fast.unwrap_or(false),
            timeout: role.timeout.as_deref().and_then(duration::parse).unwrap_or(llm::DEFAULT_TIMEOUT),
        };
        let base_url = provider.base_url.as_deref();
        match provider.kind {
            ProviderKind::Openai if provider.api.as_deref() == Some("responses") => {
                let base = base_url.unwrap_or(OPENAI_BASE_URL);
                Ok(Arc::new(Provider(OpenAiResponses::new(base, self.key(provider), &settings)?)))
            }
            ProviderKind::Openai => {
                let chat =
                    OpenAiChat::new(base_url.unwrap_or(OPENAI_BASE_URL), self.key(provider), &settings)?;
                Ok(Arc::new(Provider(chat)))
            }
            ProviderKind::Anthropic => {
                let key = self.key(provider).ok_or_else(|| {
                    ModelError::Unavailable(format!(
                        "{role_name}: the Anthropic API key ({}) is missing",
                        provider.key_env.as_deref().unwrap_or("key_env is not set")
                    ))
                })?;
                let base = base_url.unwrap_or(anthropic::DEFAULT_BASE_URL);
                Ok(Arc::new(Provider(AnthropicLlm::new(base, key, &settings)?)))
            }
            ProviderKind::Chatgpt => {
                let base = base_url.unwrap_or(chatgpt::DEFAULT_BASE_URL);
                Ok(Arc::new(Provider(ChatgptLlm::new(base, self.chatgpt_account(provider), &settings)?)))
            }
            ProviderKind::Codex => Ok(Arc::new(Provider(CodexLlm::new(provider.cmd.as_deref(), &settings)))),
            ProviderKind::Command => {
                let program = provider.cmd.as_deref().unwrap_or_default();
                Ok(Arc::new(Provider(CommandLlm::new(program, &settings)?)))
            }
            ProviderKind::Jev => Err(ModelError::NotConfigured(format!(
                "{role_name}: a jev provider only makes decisions; it cannot write text"
            ))),
        }
    }
}
