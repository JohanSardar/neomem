use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use neomem_core::ModelError;

use super::ModelHost;
use crate::chatgpt::{self, ChatgptAccount, PendingLogin};
use crate::config::{ProviderConfig, ProviderKind};

#[derive(Debug, Default)]
pub struct Accounts {
    chatgpt: Mutex<Option<Arc<ChatgptAccount>>>,
    pending: Mutex<HashMap<String, PendingLogin>>,
}

impl ModelHost {
    pub(super) fn chatgpt_account(&self, provider: &ProviderConfig) -> Arc<ChatgptAccount> {
        let mut account = self.accounts.chatgpt.lock().unwrap_or_else(PoisonError::into_inner);
        let issuer = provider.issuer.as_deref().unwrap_or(chatgpt::DEFAULT_ISSUER);
        Arc::clone(account.get_or_insert_with(|| Arc::new(ChatgptAccount::new(issuer, self.secrets.clone()))))
    }

    pub(super) fn open_sign_in(&self, name: &str) -> Result<String, ModelError> {
        let provider = self.sign_in_provider(name)?;
        let mut pending = self.accounts.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(login) = pending.get(name).filter(|login| login.is_open()) {
            return Ok(login.url.clone());
        }
        let port = provider.callback_port.unwrap_or(chatgpt::DEFAULT_CALLBACK_PORT);
        let login = self.chatgpt_account(&provider).start_login(port)?;
        let url = login.url.clone();
        pending.insert(name.to_owned(), login);
        Ok(url)
    }

    pub(super) fn await_sign_in(&self, name: &str) -> Result<String, ModelError> {
        let login = self.accounts.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(name);
        let login = login.ok_or_else(|| {
            ModelError::NotConfigured(format!("no sign-in to {name} is waiting; run neomem login {name}"))
        })?;
        login.wait()
    }

    fn sign_in_provider(&self, name: &str) -> Result<ProviderConfig, ModelError> {
        match self.config.providers.get(name) {
            Some(provider) if provider.kind == ProviderKind::Chatgpt => Ok(provider.clone()),
            Some(provider) => Err(ModelError::NotConfigured(format!(
                "{name} is a {:?} provider; only chatgpt providers sign in",
                provider.kind
            ))),
            None if name == "chatgpt" => Ok(ProviderConfig {
                kind: ProviderKind::Chatgpt,
                base_url: None,
                key_env: None,
                model: None,
                api: None,
                cmd: None,
                issuer: None,
                callback_port: None,
            }),
            None => Err(ModelError::NotConfigured(format!("there is no provider named {name}"))),
        }
    }
}
