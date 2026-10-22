use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use neomem_core::{Effort, LlmReply, LlmRequest, ModelError, ModelId};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::Secrets;
use crate::http;
use crate::llm::{LlmSettings, Wire};
use crate::openai_responses;

pub const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";
pub const DEFAULT_CALLBACK_PORT: u16 = 1455;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const SCOPE: &str = "openid profile email offline_access";
const ORIGINATOR: &str = "neomem";
const AUTH_CLAIM: &str = "https://api.openai.com/auth";
const REFRESH_MARGIN_SECS: u64 = 300;
const DEFAULT_LIFETIME_SECS: u64 = 3600;
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);
const ACCEPT_POLL: Duration = Duration::from_millis(25);
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
const SIGNED_IN_PAGE: &str =
    "<!doctype html><title>neomem</title><p>neomem is signed in to ChatGPT. You can close this tab.</p>";

const ACCESS_TOKEN: &str = "NEOMEM_CHATGPT_ACCESS_TOKEN";
const REFRESH_TOKEN: &str = "NEOMEM_CHATGPT_REFRESH_TOKEN";
const ACCOUNT_ID: &str = "NEOMEM_CHATGPT_ACCOUNT_ID";
const EXPIRES_AT: &str = "NEOMEM_CHATGPT_EXPIRES_AT";

#[derive(Clone)]
struct Tokens {
    access: String,
    refresh: String,
    account_id: String,
    expires_at: u64,
}

impl Tokens {
    fn stored_in(secrets: &Secrets) -> Option<Self> {
        let ((access, refresh), account_id) =
            secrets.get(ACCESS_TOKEN).zip(secrets.get(REFRESH_TOKEN)).zip(secrets.get(ACCOUNT_ID))?;
        let expires_at = secrets.get(EXPIRES_AT).and_then(|at| at.parse().ok()).unwrap_or(0);
        Some(Self { access, refresh, account_id, expires_at })
    }

    fn is_fresh(&self) -> bool {
        self.expires_at > now_secs() + REFRESH_MARGIN_SECS
    }

    fn credentials(&self) -> (String, String) {
        (self.access.clone(), self.account_id.clone())
    }

    fn store_in(&self, secrets: &Secrets) -> std::io::Result<()> {
        let expires_at = self.expires_at.to_string();
        secrets.store(&[
            (ACCESS_TOKEN, self.access.as_str()),
            (REFRESH_TOKEN, self.refresh.as_str()),
            (ACCOUNT_ID, self.account_id.as_str()),
            (EXPIRES_AT, expires_at.as_str()),
        ])
    }
}

impl fmt::Debug for Tokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tokens")
            .field("account_id", &self.account_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Debug)]
pub struct ChatgptAccount {
    issuer: String,
    secrets: Secrets,
    agent: ureq::Agent,
    tokens: Mutex<Option<Tokens>>,
}

#[derive(Debug)]
pub struct PendingLogin {
    pub url: String,
    outcome: Receiver<Result<String, ModelError>>,
    worker: JoinHandle<()>,
}

impl PendingLogin {
    pub fn is_open(&self) -> bool {
        !self.worker.is_finished()
    }

    pub fn wait(self) -> Result<String, ModelError> {
        self.outcome
            .recv_timeout(SIGN_IN_WAIT + Duration::from_secs(5))
            .unwrap_or_else(|_| Err(http::timed_out(SIGN_IN_WAIT)))
    }
}

impl ChatgptAccount {
    pub fn new(issuer: &str, secrets: Secrets) -> Self {
        let stored = Tokens::stored_in(&secrets);
        Self {
            issuer: issuer.trim_end_matches('/').to_owned(),
            secrets,
            agent: http::agent(TOKEN_TIMEOUT),
            tokens: Mutex::new(stored),
        }
    }

    pub fn credentials(&self) -> Result<(String, String), ModelError> {
        let mut tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(current) = tokens.as_ref().filter(|current| current.is_fresh()) {
            return Ok(current.credentials());
        }
        let newest = Tokens::stored_in(&self.secrets.reloaded()).or_else(|| tokens.clone());
        let current = newest.ok_or_else(|| {
            ModelError::NotConfigured("chatgpt: not signed in; run neomem login chatgpt".to_owned())
        })?;
        if current.is_fresh() {
            let credentials = current.credentials();
            *tokens = Some(current);
            return Ok(credentials);
        }
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", current.refresh.as_str()),
            ("client_id", CLIENT_ID),
            ("scope", "openid profile email"),
        ];
        let response: TokenResponse = http::post_form(&self.agent, &self.token_url(), &form, TOKEN_TIMEOUT)?;
        let refreshed = tokens_from(response, Some(&current));
        self.keep(&refreshed)?;
        let credentials = (refreshed.access.clone(), refreshed.account_id.clone());
        *tokens = Some(refreshed);
        Ok(credentials)
    }

    pub fn start_login(self: &Arc<Self>, callback_port: u16) -> Result<PendingLogin, ModelError> {
        let listener = TcpListener::bind(("127.0.0.1", callback_port)).map_err(|error| {
            ModelError::Unavailable(format!("cannot listen for the sign-in on port {callback_port}: {error}"))
        })?;
        let port = listener.local_addr().map_err(|error| unavailable(&error))?.port();
        let redirect_uri = format!("http://localhost:{port}/auth/callback");
        let verifier = random_token(32)?;
        let state = random_token(16)?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("response_type", "code"),
                ("client_id", CLIENT_ID),
                ("redirect_uri", redirect_uri.as_str()),
                ("scope", SCOPE),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
                ("state", state.as_str()),
                ("originator", ORIGINATOR),
            ])
            .finish();
        let url = format!("{}/oauth/authorize?{query}", self.issuer);
        let account = Arc::clone(self);
        let (sender, outcome) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = sender.send(account.complete_login(&listener, &state, &verifier, &redirect_uri));
        });
        Ok(PendingLogin { url, outcome, worker })
    }

    fn complete_login(
        &self,
        listener: &TcpListener,
        state: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<String, ModelError> {
        let code = wait_for_code(listener, state)?;
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ];
        let response: TokenResponse = http::post_form(&self.agent, &self.token_url(), &form, TOKEN_TIMEOUT)?;
        let email =
            response.id_token.as_deref().and_then(|token| claims(token)["email"].as_str().map(str::to_owned));
        let tokens = tokens_from(response, None);
        if tokens.account_id.is_empty() {
            return Err(ModelError::InvalidOutput("the sign-in named no ChatGPT account".to_owned()));
        }
        self.keep(&tokens)?;
        let account = email.unwrap_or_else(|| format!("account {}", tokens.account_id));
        *self.tokens.lock().unwrap_or_else(PoisonError::into_inner) = Some(tokens);
        Ok(account)
    }

    fn keep(&self, tokens: &Tokens) -> Result<(), ModelError> {
        tokens.store_in(&self.secrets).map_err(|error| {
            ModelError::Unavailable(format!("cannot save the ChatGPT sign-in in secrets.env: {error}"))
        })
    }

    fn token_url(&self) -> String {
        format!("{}/oauth/token", self.issuer)
    }
}

fn tokens_from(response: TokenResponse, previous: Option<&Tokens>) -> Tokens {
    let account_id = [response.id_token.as_deref(), Some(response.access_token.as_str())]
        .into_iter()
        .flatten()
        .find_map(|token| claims(token)[AUTH_CLAIM]["chatgpt_account_id"].as_str().map(str::to_owned))
        .or_else(|| previous.map(|tokens| tokens.account_id.clone()))
        .unwrap_or_default();
    let expires_at = response
        .expires_in
        .map(|seconds| now_secs() + seconds)
        .or_else(|| claims(&response.access_token)["exp"].as_u64())
        .unwrap_or_else(|| now_secs() + DEFAULT_LIFETIME_SECS);
    Tokens {
        refresh: response
            .refresh_token
            .or_else(|| previous.map(|tokens| tokens.refresh.clone()))
            .unwrap_or_default(),
        access: response.access_token,
        account_id,
        expires_at,
    }
}

fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|payload| URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, ModelError> {
    listener.set_nonblocking(true).map_err(|error| unavailable(&error))?;
    let deadline = Instant::now() + SIGN_IN_WAIT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(code) = answer_callback(stream, state)? {
                    return Ok(code);
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(ACCEPT_POLL);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Err(http::timed_out(SIGN_IN_WAIT)),
            Err(error) => return Err(unavailable(&error)),
        }
    }
}

fn answer_callback(mut stream: TcpStream, state: &str) -> Result<Option<String>, ModelError> {
    let Some(target) = read_request_target(&stream) else { return Ok(None) };
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let fields: HashMap<String, String> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let (status, page, outcome) = if path != "/auth/callback" {
        ("404 Not Found", "Not found.", Ok(None))
    } else if let Some(error) = fields.get("error") {
        let refused = Err(ModelError::Unavailable(format!("the sign-in was refused: {error}")));
        ("400 Bad Request", "The sign-in was refused.", refused)
    } else if fields.get("state").map(String::as_str) != Some(state) {
        ("400 Bad Request", "This sign-in link is not the one neomem is waiting for.", Ok(None))
    } else if let Some(code) = fields.get("code") {
        ("200 OK", SIGNED_IN_PAGE, Ok(Some(code.clone())))
    } else {
        ("400 Bad Request", "The sign-in sent no code.", Ok(None))
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
        page.len()
    );
    outcome
}

fn read_request_target(stream: &TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(CALLBACK_READ_TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut header = String::new();
    while reader.read_line(&mut header).ok()? > 2 {
        header.clear();
    }
    request_line.split_whitespace().nth(1).map(str::to_owned)
}

fn random_token(bytes: usize) -> Result<String, ModelError> {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|error| ModelError::Unavailable(format!("no randomness: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn unavailable(error: &std::io::Error) -> ModelError {
    ModelError::Unavailable(error.to_string())
}

#[derive(Debug)]
pub struct ChatgptLlm {
    agent: ureq::Agent,
    url: String,
    account: Arc<ChatgptAccount>,
    model: String,
    effort: Option<Effort>,
    timeout: Duration,
}

impl ChatgptLlm {
    pub fn new(
        base_url: &str,
        account: Arc<ChatgptAccount>,
        settings: &LlmSettings,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            agent: http::agent(settings.timeout),
            url: format!("{}/responses", base_url.trim_end_matches('/')),
            account,
            model: settings.required_model("chatgpt")?,
            effort: settings.effort,
            timeout: settings.timeout,
        })
    }
}

impl Wire for ChatgptLlm {
    fn has_schema_mode(&self) -> bool {
        true
    }

    fn send(&self, request: &LlmRequest, on_text: &mut dyn FnMut(&str)) -> Result<LlmReply, ModelError> {
        let (access, account_id) = self.account.credentials()?;
        let body = openai_responses::body(&self.model, self.effort, request)?;
        let headers = [
            ("authorization", format!("Bearer {access}")),
            ("chatgpt-account-id", account_id),
            ("openai-beta", "responses=experimental".to_owned()),
            ("originator", ORIGINATOR.to_owned()),
            ("accept", "text/event-stream".to_owned()),
        ];
        let (content_type, lines) = http::post_stream(&self.agent, &self.url, &headers, &body, self.timeout)?;
        openai_responses::read_reply(content_type, lines, self.timeout, on_text)
    }

    fn model_id(&self) -> ModelId {
        ModelId::new(format!("chatgpt:{}", self.model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in(path: &std::path::Path, access: &str, refresh: &str, expires_at: u64) {
        Secrets::load(path)
            .store(&[
                (ACCESS_TOKEN, access),
                (REFRESH_TOKEN, refresh),
                (ACCOUNT_ID, "acct"),
                (EXPIRES_AT, &expires_at.to_string()),
            ])
            .unwrap();
    }

    #[test]
    fn a_sign_in_refreshed_by_another_process_is_used_instead_of_refreshing_again() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("secrets.env");
        signed_in(&path, "expired-access", "rotated-away", now_secs() - 10);
        let account = ChatgptAccount::new("http://127.0.0.1:9", Secrets::load(&path));
        signed_in(&path, "fresh-access", "current-refresh", now_secs() + 3600);

        assert_eq!(account.credentials().unwrap(), ("fresh-access".to_owned(), "acct".to_owned()));
        assert_eq!(account.credentials().unwrap().0, "fresh-access");
    }

    #[test]
    fn the_account_comes_from_the_id_token_claims() {
        let payload =
            URL_SAFE_NO_PAD.encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-1"}}"#);
        let response = TokenResponse {
            access_token: "opaque".to_owned(),
            refresh_token: None,
            id_token: Some(format!("header.{payload}.signature")),
            expires_in: Some(60),
        };
        let previous = Tokens {
            access: String::new(),
            refresh: "kept".to_owned(),
            account_id: String::new(),
            expires_at: 0,
        };
        let tokens = tokens_from(response, Some(&previous));
        assert_eq!(tokens.account_id, "acct-1");
        assert_eq!(tokens.refresh, "kept", "a refresh without rotation keeps the old refresh token");
        assert!(tokens.expires_at > now_secs());
        assert!(!format!("{tokens:?}").contains("kept"), "tokens never print");
    }
}
