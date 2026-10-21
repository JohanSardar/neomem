use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use neomem_core::{
    Clock, Embedder, EpochMs, Judge, JudgeRole, Llm, LlmRole, ModelError, ModelId, ModelRole, ModelRoles,
    RoleAssignment, Source,
};

use crate::{ModelHost, ModelsConfig, Secrets};

#[derive(Debug)]
pub struct ReloadingHost {
    config_path: PathBuf,
    history_path: PathBuf,
    models_dir: PathBuf,
    secrets: Secrets,
    clock: Arc<dyn Clock>,
    hosts: Mutex<Hosts>,
}

#[derive(Debug, Default)]
struct Hosts {
    current_text: String,
    current: Option<Arc<ModelHost>>,
    embedders: HashMap<ModelId, Arc<ModelHost>>,
    configurations: HashMap<ModelId, String>,
}

impl ReloadingHost {
    pub fn open(
        config_path: PathBuf,
        models_dir: PathBuf,
        secrets: Secrets,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ModelError> {
        let history_path = config_path.with_file_name("model-history.json");
        let configurations: HashMap<ModelId, String> = match fs::read_to_string(&history_path) {
            Ok(text) => serde_json::from_str(&text).map_err(unavailable)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => return Err(unavailable(error)),
        };
        let host = Self {
            config_path,
            history_path,
            models_dir,
            secrets,
            clock,
            hosts: Mutex::new(Hosts::default()),
        };
        {
            let mut hosts = host.hosts.lock().unwrap_or_else(PoisonError::into_inner);
            hosts.configurations = configurations;
        }
        host.current()?;
        Ok(host)
    }

    fn build(&self, text: &str) -> Result<ModelHost, ModelError> {
        let config: ModelsConfig = toml::from_str(text).map_err(unavailable)?;
        Ok(ModelHost::new(config, self.secrets.clone(), self.models_dir.clone())
            .with_clock(Arc::clone(&self.clock)))
    }

    fn current(&self) -> Result<Arc<ModelHost>, ModelError> {
        loop {
            let text = fs::read_to_string(&self.config_path).map_err(unavailable)?;
            let previous = {
                let hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
                if text == hosts.current_text
                    && let Some(current) = &hosts.current
                {
                    return Ok(Arc::clone(current));
                }
                hosts.current.clone()
            };
            let config: ModelsConfig = toml::from_str(&text).map_err(unavailable)?;
            let mut configured = ModelHost::new(config, self.secrets.clone(), self.models_dir.clone())
                .with_clock(Arc::clone(&self.clock));
            if let Some(previous) = previous {
                configured.inherit_unchanged(&previous);
            }
            let configured = Arc::new(configured);
            let model = configured.embedder().ok().map(|embedder| embedder.model_id());
            let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
            if fs::read_to_string(&self.config_path).map_err(unavailable)? != text {
                continue;
            }
            if hosts.current_text == text
                && let Some(current) = &hosts.current
            {
                return Ok(Arc::clone(current));
            }
            if let Some(id) = model {
                hosts.embedders.insert(id.clone(), Arc::clone(&configured));
                if hosts.configurations.get(&id) != Some(&text) {
                    hosts.configurations.insert(id, text.clone());
                    let saved = serde_json::to_vec(&hosts.configurations).map_err(unavailable)?;
                    let temporary = self.history_path.with_extension("json.tmp");
                    fs::write(&temporary, saved).map_err(unavailable)?;
                    fs::rename(temporary, &self.history_path).map_err(unavailable)?;
                }
            }
            hosts.current = Some(Arc::clone(&configured));
            hosts.current_text = text;
            return Ok(configured);
        }
    }
}

impl ModelRoles for ReloadingHost {
    fn retain_embedders(&self, models: &[ModelId]) {
        let retired: Vec<_> = {
            let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
            let current = hosts.current.clone();
            hosts
                .embedders
                .extract_if(|id, host| {
                    !models.contains(id)
                        && !current.as_ref().is_some_and(|current| Arc::ptr_eq(current, host))
                })
                .map(|(_, host)| host)
                .collect()
        };
        drop(retired);
    }

    fn record_read(&self) {
        let hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        for host in hosts.embedders.values().chain(hosts.current.iter()) {
            host.record_read();
        }
    }

    fn maintain(&self, now: EpochMs) {
        if let Err(error) = self.current() {
            tracing::error!("reloading model configuration failed: {error}");
        }
        let (hosts, timeout) = {
            let hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(current) = &hosts.current else {
                return;
            };
            let timeout = current.idle_timeout();
            (hosts.embedders.values().chain(hosts.current.iter()).cloned().collect::<Vec<_>>(), timeout)
        };
        for host in hosts {
            host.expire_local_models(now, timeout);
        }
    }

    fn embedder(&self) -> Result<Arc<dyn Embedder>, ModelError> {
        self.current()?.embedder()
    }

    fn embedder_for(&self, model: &ModelId) -> Result<Arc<dyn Embedder>, ModelError> {
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let host = if let Some(host) = hosts.embedders.get(model) {
            Arc::clone(host)
        } else {
            let text = hosts
                .configurations
                .get(model)
                .ok_or_else(|| unavailable(format!("no configuration retained for {model}")))?;
            let mut historical = self.build(text)?;
            if let Some(current) = &hosts.current {
                historical.inherit_read_activity(current);
            }
            let host = Arc::new(historical);
            hosts.embedders.insert(model.clone(), Arc::clone(&host));
            host
        };
        drop(hosts);
        let embedder = host.embedder()?;
        if embedder.model_id() != *model {
            return Err(unavailable(format!("the retained configuration does not identify {model}")));
        }
        Ok(embedder)
    }

    fn use_model(&self, role: ModelRole, model: &str) -> Result<(), ModelError> {
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let text = fs::read_to_string(&self.config_path).map_err(unavailable)?;
        let mut value: toml::Value = toml::from_str(&text).map_err(unavailable)?;
        let provider_names: std::collections::HashSet<String> = value
            .get("providers")
            .and_then(toml::Value::as_table)
            .map(|providers| providers.keys().cloned().collect())
            .unwrap_or_default();
        let root = value.as_table_mut().ok_or_else(|| unavailable("config must be a table"))?;
        let models = root
            .entry("models")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| unavailable("models must be a table"))?;
        let table = models
            .entry(role.name())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| unavailable("the model role must be a table"))?;
        if std::path::Path::new(model)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        {
            if !matches!(role, ModelRole::Embedder | ModelRole::Judge | ModelRole::Checker) {
                return Err(unavailable("local GGUF assignments support the embedder, judge and checker"));
            }
            table.remove("provider");
            table.remove("model");
            table.insert("runtime".to_owned(), toml::Value::String("llama.cpp".to_owned()));
            table.insert("file".to_owned(), toml::Value::String(model.to_owned()));
        } else {
            let existing = table.get("provider").and_then(toml::Value::as_str).map(str::to_owned);
            let (provider, name) = match model.split_once('/') {
                Some((provider, name)) if provider_names.contains(provider) => (provider, name),
                _ => (
                    existing
                        .as_deref()
                        .ok_or_else(|| unavailable("name a provider/model or a local .gguf filename"))?,
                    model,
                ),
            };
            table.remove("runtime");
            table.remove("file");
            table.insert("provider".to_owned(), toml::Value::String(provider.to_owned()));
            table.insert("model".to_owned(), toml::Value::String(name.to_owned()));
        }
        let serialized = toml::to_string(&value).map_err(unavailable)?;
        let _validated = self.build(&serialized)?;
        let temporary = self.config_path.with_extension("toml.tmp");
        fs::write(&temporary, serialized).map_err(unavailable)?;
        fs::rename(temporary, &self.config_path).map_err(unavailable)?;
        hosts.current_text.clear();
        drop(hosts);
        self.current()?;
        Ok(())
    }

    fn judge(&self, role: JudgeRole) -> Result<Arc<dyn Judge>, ModelError> {
        if role == JudgeRole::Judge {
            self.record_read();
        }
        self.current()?.judge(role)
    }
    fn llm(&self, role: LlmRole, source: Option<&Source>) -> Result<Arc<dyn Llm>, ModelError> {
        if role == LlmRole::Planner {
            self.record_read();
        }
        self.current()?.llm(role, source)
    }
    fn describe(&self) -> Vec<RoleAssignment> {
        self.current().map(|host| host.describe()).unwrap_or_default()
    }
    fn start_login(&self, provider: &str) -> Result<String, ModelError> {
        self.current()?.start_login(provider)
    }
    fn finish_login(&self, provider: &str) -> Result<String, ModelError> {
        self.current()?.finish_login(provider)
    }
}

fn unavailable(error: impl std::fmt::Display) -> ModelError {
    ModelError::Unavailable(error.to_string())
}
