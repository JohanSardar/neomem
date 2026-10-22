use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use neomem_core::{Effort, Llm, LlmMessage, LlmRequest, LlmRole, ModelRoles};
use neomem_models::{ModelHost, ModelsConfig, ProviderConfig, ProviderKind, RoleConfig, Secrets};
use serde_json::{Value, json};

const PROVIDER: &str = "chatgpt";
const TIMEOUT: &str = "10m";
const BYTES_PER_TOKEN: usize = 4;
const DEFAULT_MAX_TOKENS: u32 = 4096;
const HANDLERS: usize = 32;

const PRIORITY_TIER: &str = "priority";

type Models = HashMap<(String, Option<String>, bool), Arc<dyn Llm>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let (Some(sign_in), Some(port)) = (arguments.next(), arguments.next()) else {
        return Err(
            "usage: chat_completions_bridge <secrets.env with a neomem ChatGPT sign-in> <port>".into()
        );
    };
    let sign_in = PathBuf::from(sign_in);
    let server =
        tiny_http::Server::http(("127.0.0.1", port.parse::<u16>()?)).map_err(|error| error.to_string())?;
    let json_type = tiny_http::Header::from_bytes("Content-Type", "application/json")
        .map_err(|()| "invalid content type header")?;
    let models = Mutex::new(Models::new());
    thread::scope(|scope| {
        for _ in 0..HANDLERS {
            scope.spawn(|| {
                while let Ok(request) = server.recv() {
                    if let Err(error) = answer(request, &sign_in, &models, &json_type) {
                        eprintln!("answering a request failed: {error}");
                    }
                }
            });
        }
    });
    Ok(())
}

fn answer(
    mut request: tiny_http::Request,
    sign_in: &Path,
    models: &Mutex<Models>,
    json_type: &tiny_http::Header,
) -> std::io::Result<()> {
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body)?;
    let (status, reply) = match completed(sign_in, models, &body) {
        Ok(reply) => (200, reply),
        Err(error) => (502, json!({"error": {"message": error}})),
    };
    request.respond(
        tiny_http::Response::from_string(reply.to_string())
            .with_status_code(status)
            .with_header(json_type.clone()),
    )
}

fn completed(sign_in: &Path, models: &Mutex<Models>, body: &str) -> Result<Value, String> {
    let completion: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let model = completion["model"].as_str().ok_or("the request names no model")?.to_owned();
    let effort_name = completion["reasoning_effort"].as_str().map(str::to_owned);
    let fast = completion["service_tier"].as_str() == Some(PRIORITY_TIER);
    let key = (model.clone(), effort_name.clone(), fast);
    let cached = models.lock().unwrap_or_else(PoisonError::into_inner).get(&key).map(Arc::clone);
    let llm = if let Some(llm) = cached {
        llm
    } else {
        let effort = effort_name
            .map(|name| serde_json::from_value::<Effort>(Value::String(name)))
            .transpose()
            .map_err(|error| error.to_string())?;
        let llm = chatgpt_model(sign_in, &model, effort, fast)?;
        models.lock().unwrap_or_else(PoisonError::into_inner).insert(key, Arc::clone(&llm));
        llm
    };
    let reply = llm.complete(&request_from(&completion), &mut |_| {}).map_err(|error| error.to_string())?;
    let prompt_tokens = body.len() / BYTES_PER_TOKEN;
    let completion_tokens = reply.text.len() / BYTES_PER_TOKEN;
    Ok(json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": reply.text}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens,
                  "total_tokens": prompt_tokens + completion_tokens}
    }))
}

fn request_from(completion: &Value) -> LlmRequest {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for message in completion["messages"].as_array().into_iter().flatten() {
        let content = message["content"].as_str().unwrap_or_default().to_owned();
        match message["role"].as_str() {
            Some("system" | "developer") => system.push(content),
            Some("assistant") => {
                messages.push(LlmMessage::Assistant { text: content, tool_calls: Vec::new() });
            }
            _ => messages.push(LlmMessage::User(content)),
        }
    }
    let most_tokens =
        completion["max_completion_tokens"].as_u64().or_else(|| completion["max_tokens"].as_u64());
    LlmRequest {
        system: system.join("\n\n"),
        messages,
        response_schema: None,
        tools: Vec::new(),
        max_output_tokens: most_tokens
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or(DEFAULT_MAX_TOKENS),
    }
}

fn chatgpt_model(
    sign_in: &Path,
    model: &str,
    effort: Option<Effort>,
    fast: bool,
) -> Result<Arc<dyn Llm>, String> {
    let provider = ProviderConfig {
        kind: ProviderKind::Chatgpt,
        base_url: None,
        key_env: None,
        model: None,
        api: None,
        cmd: None,
        issuer: None,
        callback_port: None,
    };
    let mut config = ModelsConfig::default();
    config.providers.insert(PROVIDER.to_owned(), provider);
    config.models.planner = Some(RoleConfig {
        provider: Some(PROVIDER.to_owned()),
        model: Some(model.to_owned()),
        effort,
        fast: Some(fast),
        timeout: Some(TIMEOUT.to_owned()),
        ..RoleConfig::default()
    });
    let host = ModelHost::new(config, Secrets::load(sign_in), PathBuf::new());
    host.llm(LlmRole::Planner, None).map_err(|error| error.to_string())
}
