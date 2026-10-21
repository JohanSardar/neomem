use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "neomem", version, about = "Long-term semantic memory for agents")]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Serve neomem's tools over MCP on stdio, relayed to the daemon")]
    Mcp,
    #[command(about = "List the tag registry, or add a tag after checking it is not a near-duplicate")]
    Tags {
        #[arg(long)]
        new: Option<String>,
    },
    #[command(about = "Attach an external reference (ext:<system>/<id>) to a fact or entity")]
    Link {
        external: String,
        #[arg(long)]
        to: String,
        #[arg(long = "type")]
        kind: String,
    },
    #[command(about = "Set up Claude Code, Codex, pi, the skill and the background service")]
    Install,
    #[command(about = "Show whether the daemon runs and what it holds")]
    Status,
    #[command(about = "Recall memories for an agent turn, or switch recall mode on or off")]
    Recall(RecallArgs),
    #[command(about = "Handle an agent hook event (called by Claude Code, Codex and pi)")]
    Hook { agent: String, event: String },
    #[command(about = "Write a record to the intake log")]
    Add(AddArgs),
    #[command(about = "Keyword search over the raw intake history, by words and dates")]
    Raw(RawArgs),
    #[command(about = "Mark an intake record as logged incorrectly")]
    Correct(CorrectArgs),
    #[command(about = "Rebuild the search index from the memory files")]
    Rebuild,
    #[command(about = "Run consolidation now")]
    Dream,
    #[command(about = "Run extraction again over the intake log")]
    Reextract {
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        from: Option<String>,
    },
    #[command(about = "Search facts: the latest version of each, ranked")]
    Search(SearchArgs),
    #[command(about = "Find the entity ID for a name or alias")]
    Resolve { name: String },
    #[command(about = "Show an entity with its aliases, facts and links")]
    Entity { name: String },
    #[command(about = "Show a memory's full file and its sources")]
    Show { id: String },
    #[command(about = "Show every version of a fact")]
    History { id: String },
    #[command(about = "Record that a memory helped")]
    Useful { id: String },
    #[command(about = "Show or swap the model for each role")]
    Models(ModelsArgs),
    #[command(about = "Sign in to a model provider")]
    Login(LoginArgs),
}

#[derive(Debug, Args)]
pub struct CorrectArgs {
    pub intake: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    pub text: String,
    #[arg(long)]
    pub k: Option<usize>,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub entities: Vec<String>,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long)]
    pub as_of: Option<String>,
    #[arg(long)]
    pub include_invalid: bool,
    #[arg(long)]
    pub include_hidden: bool,
}

#[derive(Debug, Args)]
pub struct RawArgs {
    #[arg(default_value = "")]
    pub text: String,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub action: Option<ModelsAction>,
}

#[derive(Debug, Subcommand)]
pub enum ModelsAction {
    Use {
        role: RoleArgument,
        model: String,
    },
    Test {
        role: RoleArgument,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        structured: bool,
        #[arg(long = "input")]
        inputs: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    pub provider: LoginProvider,
    #[arg(long)]
    pub no_browser: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum LoginProvider {
    Chatgpt,
}

impl LoginProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chatgpt => "chatgpt",
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum RoleArgument {
    Embedder,
    Judge,
    Checker,
    Planner,
    Extractor,
    Consolidator,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    pub content: String,
    #[arg(long)]
    pub source: String,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    #[arg(long)]
    pub start: Option<String>,
    #[arg(long)]
    pub end: Option<String>,
    #[arg(long = "entity")]
    pub entities: Vec<String>,
    #[arg(long = "ref")]
    pub source_ref: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
}

impl RoleArgument {
    pub fn model_role(self) -> neomem_core::ModelRole {
        match self {
            Self::Embedder => neomem_core::ModelRole::Embedder,
            Self::Judge => neomem_core::ModelRole::Judge,
            Self::Checker => neomem_core::ModelRole::Checker,
            Self::Planner => neomem_core::ModelRole::Planner,
            Self::Extractor => neomem_core::ModelRole::Extractor,
            Self::Consolidator => neomem_core::ModelRole::Consolidator,
        }
    }
}

#[derive(Debug, Args)]
pub struct RecallArgs {
    #[arg(value_parser = ["on", "off"])]
    pub switch: Option<String>,
    #[arg(long, value_parser = ["cc", "codex", "pi"])]
    pub agent: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub slice: Option<std::path::PathBuf>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long, default_value = "")]
    pub message: String,
    #[arg(long)]
    pub error: Option<String>,
}
