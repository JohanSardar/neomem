use crate::args::RecallArgs;
use crate::connection::DaemonConnection;
use neomem_proto::{Home, RecallQuery, Request, Response};
use std::fs;

pub fn run(home: &Home, arguments: &RecallArgs) -> anyhow::Result<Response> {
    let started = std::time::Instant::now();
    let path = home.config_path();
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut config: toml::Table = toml::from_str(&text)?;
    let project = arguments
        .project
        .as_deref()
        .map(project_key)
        .transpose()?
        .or_else(|| std::env::current_dir().ok().map(|path| path.to_string_lossy().into_owned()));
    if let Some(switch) = &arguments.switch {
        fs::create_dir_all(home.root())?;
        let recall = config
            .entry("recall")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("[recall] must be a table"))?;
        let enabled = toml::Value::Boolean(switch == "on");
        match (&arguments.project, &arguments.agent) {
            (Some(project), _) => {
                let projects = recall
                    .entry("projects")
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                    .as_table_mut()
                    .ok_or_else(|| anyhow::anyhow!("recall projects must be a table"))?;
                projects.insert(project_key(project)?, enabled);
            }
            (None, Some(agent)) => {
                let agents = recall
                    .entry("agents")
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                    .as_table_mut()
                    .ok_or_else(|| anyhow::anyhow!("recall agents must be a table"))?;
                agents.insert(agent.clone(), enabled);
            }
            (None, None) => {
                recall.insert("enabled".to_owned(), enabled);
            }
        }
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, toml::to_string_pretty(&config)?)?;
        fs::rename(temporary, path)?;
        return Ok(Response::Recalled(Vec::new()));
    }
    let agent = arguments.agent.as_deref().ok_or_else(|| anyhow::anyhow!("recall needs --agent"))?;
    let recall = config.get("recall");
    let enabled = recall
        .and_then(|recall| {
            project
                .as_deref()
                .and_then(|project| recall.get("projects")?.get(project))
                .or_else(|| recall.get("agents")?.get(agent))
                .or_else(|| recall.get("enabled"))
        })
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Ok(Response::Recalled(Vec::new()));
    }
    let slice = arguments.slice.as_ref().map(fs::read_to_string).transpose()?.unwrap_or_default();
    let session =
        arguments.session.clone().or_else(|| std::env::var("NEOMEM_SESSION_ID").ok()).unwrap_or_else(|| {
            format!(
                "cli-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        });
    let query = RecallQuery {
        turn: std::env::var("NEOMEM_TURN_ID").ok(),
        agent: agent.to_owned(),
        session,
        slice: slice.clone(),
        message: if arguments.message.is_empty() {
            slice.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("").to_owned()
        } else {
            arguments.message.clone()
        },
        error: arguments.error.clone(),
        project,
    };
    Ok(DaemonConnection::open(home)
        .and_then(|connection| {
            connection.waiting_up_to(
                std::time::Duration::from_millis(3500)
                    .saturating_sub(started.elapsed())
                    .max(std::time::Duration::from_millis(1)),
            )
        })
        .and_then(|connection| connection.call(Request::Recall(query)))
        .map_or_else(
            |_| Response::Recalled(Vec::new()),
            |response| match response {
                Response::Recalled(memories) => Response::Recalled(memories),
                _ => Response::Recalled(Vec::new()),
            },
        ))
}

fn project_key(path: &str) -> anyhow::Result<String> {
    let path = fs::canonicalize(path).or_else(|_| std::path::absolute(path))?;
    Ok(path.to_string_lossy().into_owned())
}
