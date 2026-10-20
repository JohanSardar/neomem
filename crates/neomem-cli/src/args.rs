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
    Models(ModelsArgs),
}

#[derive(Debug, Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub action: Option<ModelsAction>,
}

#[derive(Debug, Subcommand)]
pub enum ModelsAction {
    Test { role: RoleArgument },
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
