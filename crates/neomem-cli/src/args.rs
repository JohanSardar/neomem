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
    Status,
    Add(AddArgs),
    Raw(RawArgs),
    Models(ModelsArgs),
    Login(LoginArgs),
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
    Test {
        role: RoleArgument,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        structured: bool,
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
