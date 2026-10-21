use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use neomem_proto::{ModelTestReport, Response};
use neomem_testkit::{FakeRules, LlmRule, RecordedCall};
use serde_json::{Value, json};

use crate::binaries::Binaries;
use crate::isolated::IsolatedNeomem;
use crate::proof::ProofRun;

struct Route {
    provider: &'static str,
    kind: &'static str,
    model_prefix: &'static str,
    path: &'static str,
    via: Option<&'static str>,
    effort_at: &'static [&'static str],
    streams: bool,
}

impl Route {
    fn matches(&self, call: &RecordedCall) -> bool {
        call.route == self.path && self.via.is_none_or(|via| call.body["via"] == via)
    }
}

const ROUTES: &[Route] = &[
    Route {
        provider: "fake",
        kind: "openai (chat completions)",
        model_prefix: "openai:",
        path: "/v1/chat/completions",
        via: None,
        effort_at: &["reasoning_effort"],
        streams: true,
    },
    Route {
        provider: "fake_responses",
        kind: "openai (responses)",
        model_prefix: "openai:",
        path: "/v1/responses",
        via: None,
        effort_at: &["reasoning", "effort"],
        streams: true,
    },
    Route {
        provider: "fake_anthropic",
        kind: "anthropic",
        model_prefix: "anthropic:",
        path: "/v1/messages",
        via: None,
        effort_at: &["output_config", "effort"],
        streams: true,
    },
    Route {
        provider: "chatgpt",
        kind: "chatgpt",
        model_prefix: "chatgpt:",
        path: "/backend-api/codex/responses",
        via: None,
        effort_at: &["reasoning", "effort"],
        streams: true,
    },
    Route {
        provider: "fake_codex",
        kind: "codex",
        model_prefix: "codex:",
        path: "/v1/brain",
        via: Some("codex-app-server"),
        effort_at: &["wire", "effort"],
        streams: true,
    },
    Route {
        provider: "fake_command",
        kind: "command",
        model_prefix: "command:",
        path: "/v1/brain",
        via: Some("command"),
        effort_at: &["wire", "effort"],
        streams: false,
    },
];

const STREAMED_REPLY: &str = "Edge signing error fix\nEdge release date\nsigning certificate renewal";
const BAD_STRUCTURED_ANSWER: &str = "Sure! The answer is ok.";
const SIGN_IN_WAIT: Duration = Duration::from_secs(30);
const OTHER_APP_FILES: &[(&str, &str)] =
    &[("codex", ".codex/auth.json"), ("pi", ".pi/agent/auth.json"), ("hermes", ".hermes/auth.json")];

pub fn prove(proof: &mut ProofRun, binaries: &Binaries) -> anyhow::Result<()> {
    let other_apps = OtherApps::plant()?;
    let mut neomem = IsolatedNeomem::prepared(binaries)?;
    neomem.set_env("HOME", other_apps.home());
    neomem.start_daemon()?;
    let untouched = other_apps.snapshot()?;
    let stored_refresh_token = signing_in_keeps_neomems_own_token(proof, &neomem)?;
    let mut efforts = Vec::new();
    for route in ROUTES {
        switch_planner(&mut neomem, route.provider)?;
        let first_call = planner_test_answers(proof, &neomem, route)?;
        efforts.push(format!("{}={}", route.kind, effort_sent(first_call.as_ref(), route)));
        if route.provider == "fake_anthropic" {
            the_system_prompt_is_cached(proof, first_call.as_ref());
        }
        if route.streams {
            text_streams_before_the_reply_completes(proof, &neomem, route)?;
        }
        structured_answers_fit_the_schema(proof, &neomem, route)?;
        a_bad_structured_answer_is_retried_once(proof, &neomem, route)?;
    }
    proof.claim(
        "effort low reaches every provider kind in its own form",
        efforts.iter().all(|effort| effort.ends_with("=low")),
        efforts.join(", "),
    );
    an_expiring_token_is_refreshed_with_neomems_own_token(proof, &neomem, stored_refresh_token.as_deref())?;
    other_apps_are_never_touched(proof, &neomem, &other_apps, &untouched)?;
    extractor_by_source_picks_the_other_provider(proof, &mut neomem)?;
    proof.attach_logs(&neomem.logs());
    Ok(())
}

fn switch_planner(neomem: &mut IsolatedNeomem, provider: &str) -> anyhow::Result<()> {
    neomem.stop_daemon()?;
    neomem.plan_with(provider)?;
    neomem.start_daemon()
}

fn models_test(
    neomem: &IsolatedNeomem,
    arguments: &[&str],
) -> anyhow::Result<Result<ModelTestReport, String>> {
    let mut full = vec!["models", "test"];
    full.extend_from_slice(arguments);
    full.push("--json");
    let output = neomem.neomem(&full)?;
    let parsed: Option<Response> = serde_json::from_slice(&output.stdout).ok();
    Ok(match parsed {
        Some(Response::ModelTested(report)) if output.status.success() => Ok(report),
        other => Err(format!("{other:?} {}", String::from_utf8_lossy(&output.stderr).trim())),
    })
}

fn describe(outcome: &Result<ModelTestReport, String>) -> String {
    match outcome {
        Ok(report) => {
            let first =
                report.first_text_ms.map(|first| format!(", first text {first:.1} ms")).unwrap_or_default();
            format!("{} answered {:?} in {:.1} ms{first}", report.model, report.answer, report.latency_ms)
        }
        Err(problem) => problem.clone(),
    }
}

fn calls_since(neomem: &IsolatedNeomem, seen: usize, route: &Route) -> anyhow::Result<Vec<RecordedCall>> {
    Ok(neomem.fake_model_calls()?.into_iter().skip(seen).filter(|call| route.matches(call)).collect())
}

fn add_llm_rule(neomem: &IsolatedNeomem, rule: LlmRule) -> anyhow::Result<()> {
    neomem.add_fake_rules(&FakeRules { llm: vec![rule], ..FakeRules::default() })
}

fn rule(user_contains: &str, reply: &str) -> LlmRule {
    LlmRule {
        system_contains: None,
        user_contains: Some(user_contains.to_owned()),
        reply: reply.to_owned(),
        delay_ms: 0,
        delta_delay_ms: 0,
        times: Some(1),
        ..LlmRule::default()
    }
}

fn planner_test_answers(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    route: &Route,
) -> anyhow::Result<Option<RecordedCall>> {
    let seen = neomem.fake_model_calls()?.len();
    let outcome = models_test(neomem, &["planner"])?;
    let calls = calls_since(neomem, seen, route)?;
    let holds = matches!(&outcome, Ok(report) if report.model.starts_with(route.model_prefix) && report.answer == "OK")
        && !calls.is_empty();
    proof.claim(
        &format!("neomem models test planner answers through the {} provider", route.kind),
        holds,
        format!("{} via {}", describe(&outcome), route.path),
    );
    Ok(calls.into_iter().next())
}

fn effort_sent(call: Option<&RecordedCall>, route: &Route) -> String {
    let value = call.map(|call| route.effort_at.iter().fold(&call.body, |inner, key| &inner[*key]));
    value.and_then(Value::as_str).unwrap_or("none").to_owned()
}

fn the_system_prompt_is_cached(proof: &mut ProofRun, call: Option<&RecordedCall>) {
    let cache = call.map(|call| call.body["system"][0]["cache_control"]["type"].clone());
    proof.claim(
        "anthropic requests mark the system prompt for prompt caching",
        cache.as_ref().and_then(Value::as_str) == Some("ephemeral"),
        format!("{cache:?}"),
    );
}

fn text_streams_before_the_reply_completes(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    route: &Route,
) -> anyhow::Result<()> {
    add_llm_rule(neomem, LlmRule { delta_delay_ms: 60, ..rule("ping", STREAMED_REPLY) })?;
    let outcome = models_test(neomem, &["planner"])?;
    let holds = matches!(&outcome, Ok(report)
        if report.answer == STREAMED_REPLY && report.first_text_ms.is_some_and(|first| first + 300.0 < report.latency_ms));
    proof.claim(
        &format!("{}: streamed text reaches the caller before the reply completes", route.kind),
        holds,
        describe(&outcome),
    );
    Ok(())
}

fn answers_the_schema(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| {
        value
            .as_object()
            .is_some_and(|object| object.len() == 1 && object.get("answer").is_some_and(Value::is_string))
    })
}

fn structured_answers_fit_the_schema(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    route: &Route,
) -> anyhow::Result<()> {
    let outcome = models_test(neomem, &["planner", "--structured"])?;
    proof.claim(
        &format!("{}: a structured answer is valid JSON for the schema", route.kind),
        matches!(&outcome, Ok(report) if answers_the_schema(&report.answer)),
        describe(&outcome),
    );
    Ok(())
}

fn a_bad_structured_answer_is_retried_once(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    route: &Route,
) -> anyhow::Result<()> {
    add_llm_rule(neomem, rule("neomem structured test", BAD_STRUCTURED_ANSWER))?;
    let seen = neomem.fake_model_calls()?.len();
    let outcome = models_test(neomem, &["planner", "--structured"])?;
    let calls = calls_since(neomem, seen, route)?;
    let retry_quotes_the_bad_answer =
        calls.get(1).is_some_and(|retry| retry.body.to_string().contains(BAD_STRUCTURED_ANSWER));
    proof.claim(
        &format!(
            "{}: an answer that breaks the schema is retried exactly once and the retry fits",
            route.kind
        ),
        matches!(&outcome, Ok(report) if answers_the_schema(&report.answer))
            && calls.len() == 2
            && retry_quotes_the_bad_answer,
        format!("{} calls; {}", calls.len(), describe(&outcome)),
    );
    Ok(())
}

fn extractor_by_source_picks_the_other_provider(
    proof: &mut ProofRun,
    neomem: &mut IsolatedNeomem,
) -> anyhow::Result<()> {
    switch_planner(neomem, "fake")?;
    let default = models_test(neomem, &["extractor"])?;
    let seen = neomem.fake_model_calls()?.len();
    let overridden = models_test(neomem, &["extractor", "--source", "bank-digest"])?;
    let anthropic_calls = calls_since(neomem, seen, &ROUTES[2])?;
    let holds = matches!(&default, Ok(report) if report.model == "openai:fake-extractor")
        && matches!(&overridden, Ok(report) if report.model == "anthropic:claude-fake")
        && !anthropic_calls.is_empty();
    proof.claim(
        "[models.extractor_by_source] sends bank-digest extraction to its own provider and everything else to the default",
        holds,
        format!("default: {}; bank-digest: {}", describe(&default), describe(&overridden)),
    );
    Ok(())
}

fn secret_value(secrets: &str, name: &str) -> Option<String> {
    secrets
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn signing_in_keeps_neomems_own_token(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
) -> anyhow::Result<Option<String>> {
    let started = Instant::now();
    let mut login = neomem.spawn_neomem(&["login", "chatgpt", "--no-browser", "--json"])?;
    let (url, printed) = read_sign_in_address(&mut login);
    let browser = url.as_deref().map_or_else(|| Err("no sign-in address".to_owned()), act_as_the_browser);
    let finished = wait_for(login)?;
    let secrets_path = neomem.home().secrets_path();
    let secrets = fs::read_to_string(&secrets_path).unwrap_or_default();
    let mode = fs::metadata(&secrets_path)?.permissions().mode() & 0o777;
    let names = ["NEOMEM_CHATGPT_ACCESS_TOKEN", "NEOMEM_CHATGPT_REFRESH_TOKEN", "NEOMEM_CHATGPT_ACCOUNT_ID"];
    let stored = names.iter().all(|name| secret_value(&secrets, name).is_some());
    let logged_in = matches!(&finished, Some((true, Some(Response::LoggedIn { .. }))));
    proof.claim(
        "neomem login chatgpt signs in through the browser flow and keeps neomem's own token in the isolated home's secrets.env (mode 600)",
        logged_in && browser.is_ok() && stored && mode == 0o600,
        format!(
            "{:?}; browser {browser:?}; mode {mode:o}; tokens stored {stored}; {:.0} ms; {}",
            finished.and_then(|(_, response)| response),
            started.elapsed().as_secs_f64() * 1000.0,
            printed.trim()
        ),
    );
    let calls = neomem.fake_model_calls()?;
    let pkce = calls
        .iter()
        .any(|call| call.route == "/oauth/authorize" && call.body["code_challenge_method"] == "S256")
        && calls.iter().any(|call| {
            call.route == "/oauth/token"
                && call.body["grant_type"] == "authorization_code"
                && call.body["code_verifier"].as_str().is_some_and(|verifier| verifier.len() >= 43)
        });
    proof.claim(
        "the sign-in uses PKCE: an S256 challenge, then the code exchanged with its verifier",
        pkce,
        logged_in,
    );
    Ok(secret_value(&secrets, "NEOMEM_CHATGPT_REFRESH_TOKEN"))
}

fn read_sign_in_address(login: &mut Child) -> (Option<String>, String) {
    let Some(stderr) = login.stderr.take() else { return (None, String::new()) };
    let mut printed = String::new();
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        printed.push_str(&line);
        printed.push('\n');
        if line.starts_with("http") {
            return (Some(line.trim().to_owned()), printed);
        }
    }
    (None, printed)
}

fn act_as_the_browser(url: &str) -> Result<u16, String> {
    match ureq::get(url).call() {
        Ok(response) => Ok(response.status().as_u16()),
        Err(error) => Err(error.to_string()),
    }
}

fn wait_for(mut child: Child) -> anyhow::Result<Option<(bool, Option<Response>)>> {
    let deadline = Instant::now() + SIGN_IN_WAIT;
    loop {
        if let Some(status) = child.try_wait()? {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_string(&mut stdout)?;
            }
            return Ok(Some((status.success(), serde_json::from_str(stdout.trim()).ok())));
        }
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn an_expiring_token_is_refreshed_with_neomems_own_token(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    stored_at_sign_in: Option<&str>,
) -> anyhow::Result<()> {
    let calls = neomem.fake_model_calls()?;
    let refreshes: Vec<&Value> = calls
        .iter()
        .filter(|call| call.route == "/oauth/token" && call.body["grant_type"] == "refresh_token")
        .map(|call| &call.body["refresh_token"])
        .collect();
    let secrets = fs::read_to_string(neomem.home().secrets_path()).unwrap_or_default();
    let rotated = secret_value(&secrets, "NEOMEM_CHATGPT_REFRESH_TOKEN");
    let holds = stored_at_sign_in.is_some()
        && refreshes.first().and_then(|token| token.as_str()) == stored_at_sign_in
        && rotated.is_some()
        && rotated.as_deref() != stored_at_sign_in;
    proof.claim(
        "an expiring ChatGPT token is refreshed with the refresh token neomem stored, and the rotated token is saved",
        holds,
        format!("{} refreshes", refreshes.len()),
    );
    Ok(())
}

fn other_apps_are_never_touched(
    proof: &mut ProofRun,
    neomem: &IsolatedNeomem,
    other_apps: &OtherApps,
    untouched: &BTreeMap<PathBuf, FileState>,
) -> anyhow::Result<()> {
    let now = other_apps.snapshot()?;
    let sent =
        neomem.fake_model_calls()?.iter().map(|call| call.body.to_string()).collect::<Vec<_>>().join("\n");
    let leaked: Vec<&String> =
        other_apps.canaries.iter().filter(|canary| sent.contains(canary.as_str())).collect();
    proof.claim(
        "signing in, refreshing and every provider call leave the Codex, pi and Hermes sign-ins untouched and never send their tokens",
        &now == untouched && leaked.is_empty(),
        format!("{} files unchanged: {}; leaked {leaked:?}", untouched.len(), &now == untouched),
    );
    Ok(())
}

type FileState = (Vec<u8>, Option<SystemTime>);

struct OtherApps {
    root: tempfile::TempDir,
    canaries: Vec<String>,
}

impl OtherApps {
    fn plant() -> anyhow::Result<Self> {
        let root = tempfile::Builder::new().prefix("neomemctl-other-apps-").tempdir()?;
        let mut canaries = Vec::new();
        for (app, file) in OTHER_APP_FILES {
            let path = root.path().join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let access = format!("canary-{app}-access-{}", std::process::id());
            let refresh = format!("canary-{app}-refresh-{}", std::process::id());
            let auth = json!({ "tokens": { "access_token": access, "refresh_token": refresh } });
            fs::write(&path, auth.to_string())?;
            canaries.extend([access, refresh]);
        }
        Ok(Self { root, canaries })
    }

    fn home(&self) -> &Path {
        self.root.path()
    }

    fn snapshot(&self) -> anyhow::Result<BTreeMap<PathBuf, FileState>> {
        let mut files = BTreeMap::new();
        snapshot_into(self.root.path(), &mut files)?;
        Ok(files)
    }
}

fn snapshot_into(dir: &Path, files: &mut BTreeMap<PathBuf, FileState>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            files.insert(entry.path(), (Vec::new(), None));
            snapshot_into(&entry.path(), files)?;
        } else {
            files.insert(entry.path(), (fs::read(entry.path())?, metadata.modified().ok()));
        }
    }
    Ok(())
}
