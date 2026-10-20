use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::transport::{Incoming, Reply};

pub const FAKE_ACCOUNT_ID: &str = "acct-fake-neomem";
pub const FAKE_EMAIL: &str = "neomem-test@example.com";
pub const FIRST_TOKEN_LIFETIME_SECS: u64 = 120;
const REFRESHED_TOKEN_LIFETIME_SECS: u64 = 3600;

#[derive(Debug, Default)]
pub struct FakeChatgpt {
    state: Mutex<SignIns>,
}

#[derive(Debug, Default)]
struct SignIns {
    codes: HashMap<String, Grant>,
    access_tokens: HashSet<String>,
    refresh_tokens: HashSet<String>,
    issued: u64,
}

#[derive(Debug)]
struct Grant {
    challenge: String,
    redirect_uri: String,
}

impl FakeChatgpt {
    pub fn authorize(&self, query: &Value) -> Reply {
        let field = |name: &str| query[name].as_str().unwrap_or_default().to_owned();
        let complete = field("response_type") == "code"
            && field("code_challenge_method") == "S256"
            && ["client_id", "redirect_uri", "code_challenge", "state"]
                .iter()
                .all(|name| !field(name).is_empty());
        if !complete {
            return Reply::refused(400, "invalid_request: a PKCE authorization request needs every field");
        }
        let mut sign_ins = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        sign_ins.issued += 1;
        let code = format!("fake-code-{}", sign_ins.issued);
        let grant = Grant { challenge: field("code_challenge"), redirect_uri: field("redirect_uri") };
        sign_ins.codes.insert(code.clone(), grant);
        let back = form_urlencoded::Serializer::new(String::new())
            .append_pair("code", &code)
            .append_pair("state", &field("state"))
            .finish();
        Reply::Redirect(format!("{}?{back}", field("redirect_uri")))
    }

    pub fn token(&self, form: &Value) -> Reply {
        let field = |name: &str| form[name].as_str().unwrap_or_default();
        let mut sign_ins = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match field("grant_type") {
            "authorization_code" => {
                let Some(grant) = sign_ins.codes.remove(field("code")) else {
                    return Reply::refused(400, "invalid_grant: unknown or used code");
                };
                let proven = URL_SAFE_NO_PAD.encode(Sha256::digest(field("code_verifier").as_bytes()))
                    == grant.challenge;
                if !proven || grant.redirect_uri != field("redirect_uri") || field("client_id").is_empty() {
                    return Reply::refused(400, "invalid_grant: the verifier or redirect does not match");
                }
                Reply::ok(sign_ins.issue(FIRST_TOKEN_LIFETIME_SECS))
            }
            "refresh_token" => {
                if !sign_ins.refresh_tokens.remove(field("refresh_token")) {
                    return Reply::refused(400, "invalid_grant: unknown or rotated refresh token");
                }
                Reply::ok(sign_ins.issue(REFRESHED_TOKEN_LIFETIME_SECS))
            }
            _ => Reply::refused(400, "unsupported_grant_type"),
        }
    }

    pub fn allows(&self, incoming: &Incoming) -> bool {
        let token =
            incoming.header("authorization").and_then(|value| value.strip_prefix("Bearer ")).unwrap_or("");
        let sign_ins = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        sign_ins.access_tokens.contains(token)
            && incoming.header("chatgpt-account-id") == Some(FAKE_ACCOUNT_ID)
    }
}

impl SignIns {
    fn issue(&mut self, lifetime: u64) -> Value {
        self.issued += 1;
        let expires = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() + lifetime;
        let auth = json!({ "chatgpt_account_id": FAKE_ACCOUNT_ID, "chatgpt_plan_type": "plus" });
        let access = jwt(&json!({ "exp": expires, "jti": self.issued, "https://api.openai.com/auth": auth }));
        let id_token =
            jwt(&json!({ "exp": expires, "email": FAKE_EMAIL, "https://api.openai.com/auth": auth }));
        let refresh = format!("fake-chatgpt-refresh-{}", self.issued);
        self.access_tokens.insert(access.clone());
        self.refresh_tokens.insert(refresh.clone());
        json!({ "access_token": access, "refresh_token": refresh, "id_token": id_token,
            "token_type": "Bearer", "expires_in": lifetime })
    }
}

fn jwt(claims: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    format!("{header}.{}.fake-signature", URL_SAFE_NO_PAD.encode(claims.to_string()))
}
