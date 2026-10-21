use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use neomem_proto::{Home, InstallOutcome, InstallReport, InstallStep, Response, publish_file};

use crate::codex_hooks;
use crate::connection::stop_daemon;
use crate::service::{Service, described, quietly, succeeded};

const MARKETPLACE: &str = "neomem";
const PLUGIN: &str = "neomem@neomem";

struct Template {
    path: &'static str,
    text: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template {
        path: ".claude-plugin/marketplace.json",
        text: include_str!("../../../integrations/.claude-plugin/marketplace.json"),
    },
    Template {
        path: ".agents/plugins/marketplace.json",
        text: include_str!("../../../integrations/.agents/plugins/marketplace.json"),
    },
    Template {
        path: "agents-plugin/.claude-plugin/plugin.json",
        text: include_str!("../../../integrations/agents-plugin/.claude-plugin/plugin.json"),
    },
    Template {
        path: "agents-plugin/.codex-plugin/plugin.json",
        text: include_str!("../../../integrations/agents-plugin/.codex-plugin/plugin.json"),
    },
    Template {
        path: "agents-plugin/.mcp.json",
        text: include_str!("../../../integrations/agents-plugin/.mcp.json"),
    },
    Template {
        path: "agents-plugin/hooks/claude-code.json",
        text: include_str!("../../../integrations/agents-plugin/hooks/claude-code.json"),
    },
    Template {
        path: "agents-plugin/hooks/codex.json",
        text: include_str!("../../../integrations/agents-plugin/hooks/codex.json"),
    },
    Template { path: "pi/package.json", text: include_str!("../../../integrations/pi/package.json") },
    Template { path: "pi/index.js", text: include_str!("../../../integrations/pi/index.js") },
    Template { path: "pi/socket.js", text: include_str!("../../../integrations/pi/socket.js") },
    Template { path: "pi/README.md", text: include_str!("../../../integrations/pi/README.md") },
    Template {
        path: "skills/neomem/SKILL.md",
        text: include_str!("../../../integrations/skills/neomem/SKILL.md"),
    },
];

struct Places {
    user_home: PathBuf,
    claude: PathBuf,
}

impl Places {
    fn from_env() -> anyhow::Result<Self> {
        let user_home =
            env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        let claude =
            env::var_os("CLAUDE_CONFIG_DIR").map_or_else(|| user_home.join(".claude"), PathBuf::from);
        Ok(Self { user_home, claude })
    }
}

struct Filled {
    neomem: String,
    home: String,
    hook: String,
}

pub fn run(home: &Home) -> anyhow::Result<Response> {
    let places = Places::from_env()?;
    let root = home.root().join("integrations");
    let filled = filled(home)?;
    let mut steps = vec![write_integrations(&root, &filled)];
    steps.push(link_skill("skill for Codex and pi", &places.user_home.join(".agents/skills/neomem"), &root));
    steps.push(link_skill("skill for Claude Code", &places.claude.join("skills/neomem"), &root));
    steps.push(claude_code(&root));
    steps.push(codex(&root, &places.user_home));
    steps.push(pi(&root));
    steps.push(service(home));
    Ok(Response::Installed(InstallReport { steps }))
}

fn filled(home: &Home) -> anyhow::Result<Filled> {
    let neomem = env::current_exe()?.display().to_string();
    let home = home.root().display().to_string();
    let hook = format!("NEOMEM_HOME={} {} hook", shell_quoted(&home), shell_quoted(&neomem));
    Ok(Filled { neomem, home, hook })
}

fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn inside_json_string(text: &str) -> String {
    let quoted = serde_json::Value::String(text.to_owned()).to_string();
    quoted[1..quoted.len() - 1].to_owned()
}

fn fill(template: &Template, filled: &Filled) -> String {
    let fill_value = |text: &str| {
        if Path::new(template.path).extension().is_some_and(|extension| extension == "json") {
            inside_json_string(text)
        } else {
            text.to_owned()
        }
    };
    template
        .text
        .replace("{{hook}}", &fill_value(&filled.hook))
        .replace("{{neomem}}", &fill_value(&filled.neomem))
        .replace("{{home}}", &fill_value(&filled.home))
}

fn step(part: &str, outcome: InstallOutcome, detail: impl Into<String>) -> InstallStep {
    InstallStep { part: part.to_owned(), outcome, detail: detail.into() }
}

fn write_integrations(root: &Path, filled: &Filled) -> InstallStep {
    let written: io::Result<()> = TEMPLATES.iter().try_for_each(|template| {
        let path = root.join(template.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        publish_file(&path, fill(template, filled))
    });
    match written {
        Ok(()) => step("integrations", InstallOutcome::Done, root.display().to_string()),
        Err(error) => step("integrations", InstallOutcome::Failed, error.to_string()),
    }
}

fn link_skill(part: &str, link: &Path, root: &Path) -> InstallStep {
    let target = root.join("skills/neomem");
    match fs::read_link(link) {
        Ok(existing) if existing == target => {
            return step(part, InstallOutcome::Done, link.display().to_string());
        }
        Ok(existing) => {
            return step(
                part,
                InstallOutcome::Skipped,
                format!("{} already points to {}", link.display(), existing.display()),
            );
        }
        Err(_) if link.exists() => {
            return step(part, InstallOutcome::Skipped, format!("{} already exists", link.display()));
        }
        Err(_) => {}
    }
    let linked = link.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| symlink(&target, link));
    match linked {
        Ok(()) => step(part, InstallOutcome::Done, link.display().to_string()),
        Err(error) => step(part, InstallOutcome::Failed, error.to_string()),
    }
}

fn installed(program: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|path| env::split_paths(&path).any(|directory| directory.join(program).is_file()))
}

fn agent_plugin(
    part: &str,
    program: &str,
    add_marketplace: &[&str],
    refresh_marketplace: &[&str],
    install: &[&str],
) -> InstallStep {
    if !installed(program) {
        return step(part, InstallOutcome::Skipped, format!("{program} is not installed"));
    }
    let added = quietly(program, add_marketplace);
    if !succeeded(&added) {
        let refreshed = quietly(program, refresh_marketplace);
        if !succeeded(&refreshed) {
            return step(part, InstallOutcome::Failed, described(&refreshed));
        }
    }
    let plugin = quietly(program, install);
    if succeeded(&plugin) {
        step(part, InstallOutcome::Done, format!("plugin {PLUGIN}"))
    } else {
        step(part, InstallOutcome::Failed, described(&plugin))
    }
}

fn claude_code(root: &Path) -> InstallStep {
    let root = root.display().to_string();
    agent_plugin(
        "Claude Code",
        "claude",
        &["plugin", "marketplace", "add", &root],
        &["plugin", "marketplace", "update", MARKETPLACE],
        &["plugin", "install", PLUGIN],
    )
}

fn codex(root: &Path, user_home: &Path) -> InstallStep {
    let root = root.display().to_string();
    let plugin = agent_plugin(
        "Codex",
        "codex",
        &["plugin", "marketplace", "add", &root],
        &["plugin", "marketplace", "upgrade", MARKETPLACE],
        &["plugin", "add", PLUGIN],
    );
    if plugin.outcome != InstallOutcome::Done {
        return plugin;
    }
    match codex_hooks::trust(PLUGIN, user_home) {
        Ok(count) => step("Codex", InstallOutcome::Done, format!("plugin {PLUGIN}, {count} hooks trusted")),
        Err(problem) => step("Codex", InstallOutcome::Failed, problem),
    }
}

fn pi(root: &Path) -> InstallStep {
    if !installed("pi") {
        return step("pi", InstallOutcome::Skipped, "pi is not installed");
    }
    let package = root.join("pi");
    let dependencies = Command::new("npm")
        .args(["install", "--omit=dev", "--ignore-scripts", "--no-audit", "--no-fund"])
        .current_dir(&package)
        .stdin(Stdio::null())
        .output();
    if !succeeded(&dependencies) {
        return step("pi", InstallOutcome::Failed, format!("npm install: {}", described(&dependencies)));
    }
    let added = quietly("pi", &["install", &package.display().to_string()]);
    if succeeded(&added) {
        step("pi", InstallOutcome::Done, format!("package {}", package.display()))
    } else {
        step("pi", InstallOutcome::Failed, described(&added))
    }
}

fn service(home: &Home) -> InstallStep {
    let Some(service) = Service::for_this_machine() else {
        return step("service", InstallOutcome::Skipped, "HOME is not set");
    };
    if !service.is_running() {
        stop_daemon(home);
    }
    match service.install(home) {
        Ok(()) => step("service", InstallOutcome::Done, service.file().display().to_string()),
        Err(problem) => step("service", InstallOutcome::Failed, problem),
    }
}
