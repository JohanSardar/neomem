use clap::{Parser, Subcommand};

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
}
