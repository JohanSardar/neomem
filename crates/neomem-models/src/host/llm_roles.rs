use std::sync::Arc;

use neomem_core::{Llm, LlmRole, ModelError, Source};

use super::ModelHost;
use crate::config::{ProviderKind, RoleConfig};
use crate::duration;
use crate::llm::{self, LlmSettings, Provider};
use crate::openai_chat::OpenAiChat;

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
            timeout: role.timeout.as_deref().and_then(duration::parse).unwrap_or(llm::DEFAULT_TIMEOUT),
        };
        let base_url = provider.base_url.as_deref();
        match provider.kind {
            ProviderKind::Openai => {
                let chat =
                    OpenAiChat::new(base_url.unwrap_or(OPENAI_BASE_URL), self.key(provider), &settings)?;
                Ok(Arc::new(Provider(chat)))
            }
            other => Err(ModelError::NotConfigured(format!(
                "{role_name}: provider kind {other:?} cannot write text yet"
            ))),
        }
    }
}
