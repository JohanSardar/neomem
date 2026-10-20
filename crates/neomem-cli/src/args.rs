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
