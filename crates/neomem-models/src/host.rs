use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use neomem_core::{
    Embedder, Judge, JudgeRole, Llm, LlmRole, ModelError, ModelRole, ModelRoles, RoleAssignment, Source,
};

use crate::config::{ModelsConfig, ProviderConfig, ProviderKind, RoleConfig};
use crate::fallback_judge::FallbackJudge;
use crate::jev::{self, JevJudge};
use crate::llama_embedder::LlamaEmbedder;
use crate::llama_reranker::LazyReranker;
use crate::openai_embeddings::OpenAiEmbedder;
use crate::query_cache::{QUERY_CACHE_ENTRIES, QueryCache};
use crate::{Secrets, duration};

mod llm_roles;
mod sign_in;

const LLAMA_CPP: &str = "llama.cpp";

#[derive(Debug)]
pub struct ModelHost {
    config: ModelsConfig,
    secrets: Secrets,
    models_dir: PathBuf,
    ready: Mutex<ReadyModels>,
    accounts: sign_in::Accounts,
}

#[derive(Debug, Default)]
struct ReadyModels {
    embedder: Option<Arc<dyn Embedder>>,
    judges: HashMap<JudgeRole, Arc<dyn Judge>>,
    llms: HashMap<(LlmRole, Option<String>), Arc<dyn Llm>>,
    rerankers: HashMap<PathBuf, Arc<LazyReranker>>,
}

impl ReadyModels {
    fn reranker(&mut self, path: PathBuf) -> Arc<dyn Judge> {
        let shared = self.rerankers.entry(path.clone()).or_insert_with(|| Arc::new(LazyReranker::at(path)));
        Arc::clone(shared) as Arc<dyn Judge>
    }
}

impl ModelHost {
    pub fn new(config: ModelsConfig, secrets: Secrets, models_dir: PathBuf) -> Self {
        Self {
            config,
            secrets,
            models_dir,
            ready: Mutex::new(ReadyModels::default()),
            accounts: sign_in::Accounts::default(),
        }
    }

    pub fn models_dir(&self) -> &PathBuf {
        &self.models_dir
    }

    fn provider(&self, role: &RoleConfig, role_name: &str) -> Result<&ProviderConfig, ModelError> {
        let name = role
            .provider
            .as_deref()
            .ok_or_else(|| ModelError::NotConfigured(format!("{role_name}: no provider or runtime")))?;
        self.config
            .providers
            .get(name)
            .ok_or_else(|| ModelError::NotConfigured(format!("{role_name}: unknown provider {name}")))
    }

    fn local_model_path(&self, role: &RoleConfig, role_name: &str) -> Result<Option<PathBuf>, ModelError> {
        match role.runtime.as_deref() {
            None => Ok(None),
            Some(LLAMA_CPP) => {
                role.file.as_deref().map(|file| Some(self.models_dir.join(file))).ok_or_else(|| {
                    ModelError::NotConfigured(format!("{role_name}: llama.cpp needs a model file"))
                })
            }
            Some(other) => Err(ModelError::NotConfigured(format!("{role_name}: unknown runtime {other}"))),
        }
    }

    fn key(&self, provider: &ProviderConfig) -> Option<String> {
        provider.key_env.as_deref().and_then(|name| self.secrets.get(name))
    }

    fn describe_role(&self, role: ModelRole) -> String {
        let roles = &self.config.models;
        let configured = match role {
            ModelRole::Embedder => roles.embedder.as_ref(),
            ModelRole::Judge => roles.judge.as_ref(),
            ModelRole::Checker => roles.checker.as_ref(),
            ModelRole::Planner => roles.planner.as_ref(),
            ModelRole::Extractor => roles.extractor.as_ref(),
            ModelRole::Consolidator => roles.consolidator.as_ref(),
        };
        configured.map_or_else(|| "not configured".to_owned(), describe_role_config)
    }

    fn build_embedder(&self) -> Result<Arc<dyn Embedder>, ModelError> {
        let role = self
            .config
            .models
            .embedder
            .as_ref()
            .ok_or_else(|| ModelError::NotConfigured("embedder".into()))?;
        if let Some(path) = self.local_model_path(role, "embedder")? {
            return Ok(Arc::new(LlamaEmbedder::open(&path)?));
        }
        let provider = self.provider(role, "embedder")?;
        match provider.kind {
            ProviderKind::Openai => {
                let base_url = provider.base_url.as_deref().unwrap_or("https://api.openai.com/v1");
                let model =
                    role.model.as_deref().or(provider.model.as_deref()).unwrap_or("text-embedding-3-small");
                Ok(Arc::new(OpenAiEmbedder::connect(base_url, self.key(provider), model)?))
            }
            other => {
                Err(ModelError::NotConfigured(format!("embedder: provider kind {other:?} cannot embed")))
            }
        }
    }

    fn build_judge(
        &self,
        role_name: JudgeRole,
        ready: &mut ReadyModels,
    ) -> Result<Arc<dyn Judge>, ModelError> {
        let roles = &self.config.models;
        let role = match role_name {
            JudgeRole::Judge => roles.judge.as_ref(),
            JudgeRole::Checker => roles.checker.as_ref(),
        }
        .ok_or_else(|| ModelError::NotConfigured(format!("{role_name:?}")))?;
        if let Some(path) = self.local_model_path(role, "judge")? {
            return Ok(ready.reranker(path));
        }
        let primary = self.build_remote_judge(role);
        let Some(fallback) = role.fallback.as_deref() else { return primary };
        let path = self.local_model_path(fallback, "judge fallback")?.ok_or_else(|| {
            ModelError::NotConfigured("judge fallback: only a llama.cpp reranker can be the fallback".into())
        })?;
        Ok(Arc::new(FallbackJudge::new(primary, ready.reranker(path))))
    }

    fn build_remote_judge(&self, role: &RoleConfig) -> Result<Arc<dyn Judge>, ModelError> {
        let provider = self.provider(role, "judge")?;
        match provider.kind {
            ProviderKind::Jev => {
                let key = self
                    .key(provider)
                    .ok_or_else(|| ModelError::Unavailable("the Jev API key is missing".into()))?;
                let model = provider.model.as_deref().unwrap_or("jev-latest");
                let timeout =
                    role.timeout.as_deref().and_then(duration::parse).unwrap_or(jev::DEFAULT_TIMEOUT);
                Ok(Arc::new(JevJudge::new(provider.base_url.as_deref(), key, model, timeout)))
            }
            other => {
                Err(ModelError::NotConfigured(format!("judge: provider kind {other:?} is not wired yet")))
            }
        }
    }
}

impl ModelRoles for ModelHost {
    fn embedder(&self) -> Result<Arc<dyn Embedder>, ModelError> {
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(embedder) = &ready.embedder {
            return Ok(Arc::clone(embedder));
        }
        let embedder: Arc<dyn Embedder> =
            Arc::new(QueryCache::around(self.build_embedder()?, QUERY_CACHE_ENTRIES));
        ready.embedder = Some(Arc::clone(&embedder));
        Ok(embedder)
    }

    fn judge(&self, role: JudgeRole) -> Result<Arc<dyn Judge>, ModelError> {
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(judge) = ready.judges.get(&role) {
            return Ok(Arc::clone(judge));
        }
        let judge = self.build_judge(role, &mut ready)?;
        ready.judges.insert(role, Arc::clone(&judge));
        Ok(judge)
    }

    fn llm(&self, role: LlmRole, source: Option<&Source>) -> Result<Arc<dyn Llm>, ModelError> {
        let (override_source, role_config) = self.llm_role(role, source)?;
        let key = (role, override_source);
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(llm) = ready.llms.get(&key) {
            return Ok(Arc::clone(llm));
        }
        let llm = self.build_llm(&format!("{role:?}").to_lowercase(), role_config)?;
        ready.llms.insert(key, Arc::clone(&llm));
        Ok(llm)
    }

    fn describe(&self) -> Vec<RoleAssignment> {
        ModelRole::ALL
            .iter()
            .map(|role| RoleAssignment { role: *role, assignment: self.describe_role(*role) })
            .collect()
    }

    fn start_login(&self, provider: &str) -> Result<String, ModelError> {
        self.open_sign_in(provider)
    }

    fn finish_login(&self, provider: &str) -> Result<String, ModelError> {
        self.await_sign_in(provider)
    }
}

fn describe_role_config(role: &RoleConfig) -> String {
    let base = match (&role.runtime, &role.provider) {
        (Some(runtime), _) => format!("{runtime} {}", role.file.as_deref().unwrap_or("")),
        (None, Some(provider)) => format!("{provider} {}", role.model.as_deref().unwrap_or("")),
        (None, None) => "incomplete".to_owned(),
    };
    let effort = role.effort.map(|effort| format!(" effort {effort:?}").to_lowercase()).unwrap_or_default();
    let fallback = role
        .fallback
        .as_deref()
        .map(|fallback| format!(" (fallback {})", describe_role_config(fallback)))
        .unwrap_or_default();
    format!("{}{effort}{fallback}", base.trim_end())
}
