use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use neomem_core::{
    Clock, Embedder, EpochMs, Judge, JudgeRole, Llm, LlmRole, ModelError, ModelRole, ModelRoles,
    RoleAssignment, Source,
};

use crate::config::{ModelsConfig, ProviderConfig, ProviderKind, RoleConfig};
use crate::fallback_judge::FallbackJudge;
use crate::jev::{self, JevJudge};
use crate::lazy_embedder::LazyEmbedder;
use crate::llama_reranker::LazyReranker;
use crate::local_slot::WallClock;
use crate::openai_embeddings::OpenAiEmbedder;
use crate::query_cache::{QUERY_CACHE_ENTRIES, QueryCache};
use crate::{Secrets, duration};

mod llm_roles;
mod sign_in;

const LLAMA_CPP: &str = "llama.cpp";

#[derive(Debug)]
pub struct ModelHost {
    config: ModelsConfig,
    clock: Arc<dyn Clock>,
    last_read: Arc<AtomicI64>,
    secrets: Secrets,
    models_dir: PathBuf,
    ready: Mutex<ReadyModels>,
    accounts: Arc<sign_in::Accounts>,
}

#[derive(Debug, Default)]
struct ReadyModels {
    embedder: Option<Arc<dyn Embedder>>,
    local_embedder: Option<Arc<LazyEmbedder>>,
    judges: HashMap<JudgeRole, Arc<dyn Judge>>,
    llms: HashMap<(LlmRole, Option<String>), Arc<dyn Llm>>,
    rerankers: HashMap<PathBuf, Arc<LazyReranker>>,
}

impl ReadyModels {
    fn reranker(&mut self, path: PathBuf, last_read: Arc<AtomicI64>) -> Arc<dyn Judge> {
        let shared = self
            .rerankers
            .entry(path.clone())
            .or_insert_with(|| Arc::new(LazyReranker::with_activity(path, last_read)));
        Arc::clone(shared) as Arc<dyn Judge>
    }
}

impl ModelHost {
    pub fn new(config: ModelsConfig, secrets: Secrets, models_dir: PathBuf) -> Self {
        Self {
            config,
            clock: Arc::new(WallClock),
            last_read: Arc::new(AtomicI64::new(i64::MIN)),
            secrets,
            models_dir,
            ready: Mutex::new(ReadyModels::default()),
            accounts: Arc::new(sign_in::Accounts::default()),
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub(crate) fn inherit_unchanged(&mut self, previous: &Self) {
        self.accounts = Arc::clone(&previous.accounts);
        self.inherit_read_activity(previous);
        let same_providers = self.config.providers == previous.config.providers;
        let old = previous.ready.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if self.config.models.embedder == previous.config.models.embedder
            && (same_providers
                || self.config.models.embedder.as_ref().is_some_and(|role| role.runtime.is_some()))
        {
            ready.embedder.clone_from(&old.embedder);
            ready.local_embedder.clone_from(&old.local_embedder);
        }
        for (path, reranker) in &old.rerankers {
            let referenced = [self.config.models.judge.as_ref(), self.config.models.checker.as_ref()]
                .into_iter()
                .flatten()
                .flat_map(|role| std::iter::once(role).chain(role.fallback.as_deref()))
                .any(|role| {
                    role.runtime.as_deref() == Some(LLAMA_CPP)
                        && role.file.as_ref().is_some_and(|file| self.models_dir.join(file) == *path)
                });
            if referenced {
                ready.rerankers.insert(path.clone(), Arc::clone(reranker));
            }
        }
        for (role, judge) in &old.judges {
            let unchanged = match role {
                JudgeRole::Judge => self.config.models.judge == previous.config.models.judge,
                JudgeRole::Checker => self.config.models.checker == previous.config.models.checker,
            };
            if unchanged && same_providers {
                ready.judges.insert(*role, Arc::clone(judge));
            }
        }
        for ((role, source), llm) in &old.llms {
            let unchanged = match role {
                LlmRole::Planner => self.config.models.planner == previous.config.models.planner,
                LlmRole::Extractor => {
                    self.config.models.extractor == previous.config.models.extractor
                        && self.config.models.extractor_by_source
                            == previous.config.models.extractor_by_source
                }
                LlmRole::Consolidator => {
                    self.config.models.consolidator == previous.config.models.consolidator
                }
            };
            if unchanged && same_providers {
                ready.llms.insert((*role, source.clone()), Arc::clone(llm));
            }
        }
    }

    pub(crate) fn inherit_read_activity(&mut self, previous: &Self) {
        self.last_read = Arc::clone(&previous.last_read);
    }

    pub(crate) fn idle_timeout(&self) -> Duration {
        self.config.models.idle_unload.as_deref().and_then(duration::parse).unwrap_or(Duration::from_mins(15))
    }

    pub(crate) fn expire_local_models(&self, now: EpochMs, timeout: Duration) {
        let ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(embedder) = &ready.local_embedder {
            embedder.maintain(now, timeout);
        }
        for reranker in ready.rerankers.values() {
            reranker.maintain(now, timeout);
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

    fn build_remote_embedder(&self) -> Result<Arc<dyn Embedder>, ModelError> {
        let role = self
            .config
            .models
            .embedder
            .as_ref()
            .ok_or_else(|| ModelError::NotConfigured("embedder".into()))?;
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
            return Ok(ready.reranker(path, Arc::clone(&self.last_read)));
        }
        let primary = self.build_remote_judge(role);
        let Some(fallback) = role.fallback.as_deref() else { return primary };
        let path = self.local_model_path(fallback, "judge fallback")?.ok_or_else(|| {
            ModelError::NotConfigured("judge fallback: only a llama.cpp reranker can be the fallback".into())
        })?;
        Ok(Arc::new(FallbackJudge::new(primary, ready.reranker(path, Arc::clone(&self.last_read)))))
    }

    fn build_remote_judge(&self, role: &RoleConfig) -> Result<Arc<dyn Judge>, ModelError> {
        let provider = self.provider(role, "judge")?;
        match provider.kind {
            ProviderKind::Jev => {
                let key = self
                    .key(provider)
                    .ok_or_else(|| ModelError::Unavailable("the Jev API key is missing".into()))?;
                let model = role.model.as_deref().or(provider.model.as_deref()).unwrap_or("jev-latest");
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
    fn record_read(&self) {
        self.last_read.store(self.clock.now().as_millis(), Ordering::SeqCst);
    }

    fn maintain(&self, now: EpochMs) {
        self.expire_local_models(now, self.idle_timeout());
    }

    fn embedder(&self) -> Result<Arc<dyn Embedder>, ModelError> {
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(embedder) = &ready.embedder {
            return Ok(Arc::clone(embedder));
        }
        let local_path = self
            .config
            .models
            .embedder
            .as_ref()
            .map(|role| self.local_model_path(role, "embedder"))
            .transpose()?
            .flatten();
        let embedder: Arc<dyn Embedder> = if let Some(path) = local_path {
            let local = Arc::new(LazyEmbedder::at(path, Arc::clone(&self.last_read))?);
            ready.local_embedder = Some(Arc::clone(&local));
            local
        } else {
            Arc::new(QueryCache::around(self.build_remote_embedder()?, QUERY_CACHE_ENTRIES))
        };
        ready.embedder = Some(Arc::clone(&embedder));
        Ok(embedder)
    }

    fn judge(&self, role: JudgeRole) -> Result<Arc<dyn Judge>, ModelError> {
        if role == JudgeRole::Judge {
            self.record_read();
        }
        let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(judge) = ready.judges.get(&role) {
            return Ok(Arc::clone(judge));
        }
        let judge = self.build_judge(role, &mut ready)?;
        ready.judges.insert(role, Arc::clone(&judge));
        Ok(judge)
    }

    fn llm(&self, role: LlmRole, source: Option<&Source>) -> Result<Arc<dyn Llm>, ModelError> {
        if role == LlmRole::Planner {
            self.record_read();
        }
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
            .map(|role| RoleAssignment {
                role: *role,
                assignment: {
                    let ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
                    let loaded = match role {
                        ModelRole::Embedder => self
                            .config
                            .models
                            .embedder
                            .as_ref()
                            .filter(|role| role.runtime.as_deref() == Some(LLAMA_CPP))
                            .map(|_| ready.local_embedder.as_ref().is_some_and(|local| local.is_loaded())),
                        ModelRole::Judge | ModelRole::Checker => match role {
                            ModelRole::Checker => self.config.models.checker.as_ref(),
                            _ => self.config.models.judge.as_ref(),
                        }
                        .and_then(|configured| {
                            configured.file.as_ref().or_else(|| {
                                configured.fallback.as_ref().and_then(|fallback| fallback.file.as_ref())
                            })
                        })
                        .map(|file| {
                            ready
                                .rerankers
                                .get(&self.models_dir.join(file))
                                .is_some_and(|local| local.is_loaded())
                        }),
                        _ => None,
                    };
                    let state = loaded.map_or("", |loaded| if loaded { "loaded " } else { "unloaded " });
                    format!("{state}{}", self.describe_role(*role))
                },
            })
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
    let effort = role.effort.map(|effort| format!(" effort {}", effort.name())).unwrap_or_default();
    let fast = if role.fast == Some(true) { " fast" } else { "" };
    let fallback = role
        .fallback
        .as_deref()
        .map(|fallback| format!(" (fallback {})", describe_role_config(fallback)))
        .unwrap_or_default();
    format!("{}{effort}{fast}{fallback}", base.trim_end())
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use crate::local_slot::LocalSlot;

    #[derive(Debug)]
    struct TestClock(AtomicI64);

    impl Clock for TestClock {
        fn now(&self) -> EpochMs {
            EpochMs::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    #[test]
    fn a_late_model_inherits_reads_before_and_after_it_opens() {
        let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
        let active = ModelHost::new(ModelsConfig::default(), Secrets::default(), PathBuf::new())
            .with_clock(clock.clone());
        active.record_read();
        let mut historical = ModelHost::new(ModelsConfig::default(), Secrets::default(), PathBuf::new())
            .with_clock(clock.clone());
        historical.inherit_read_activity(&active);
        let slot = LocalSlot::new(Arc::clone(&historical.last_read));
        slot.run(|| Ok(7), |value| Ok(*value)).unwrap();
        slot.maintain(clock.now(), Duration::from_mins(15));
        assert!(slot.is_loaded());
        clock.0.store(1_100_000, Ordering::SeqCst);
        active.record_read();
        slot.maintain(EpochMs::from_millis(1_940_000), Duration::from_mins(15));
        assert!(slot.is_loaded());
    }
}
