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
    Tags {
        #[arg(long)]
        new: Option<String>,
    },
    Link {
        external: String,
        #[arg(long)]
        to: String,
        #[arg(long = "type")]
        kind: String,
    },
    Status,
    Recall(RecallArgs),
    Hook {
        agent: String,
        event: String,
    },
    Add(AddArgs),
    Raw(RawArgs),
    Correct(CorrectArgs),
    Rebuild,
    Dream,
    Reextract {
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        from: Option<String>,
    },
    Search(SearchArgs),
    Resolve {
        name: String,
    },
    Entity {
        name: String,
    },
    Show {
        id: String,
    },
    History {
        id: String,
    },
    Useful {
        id: String,
    },
    Models(ModelsArgs),
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
