mod attachments;
mod browser;
mod cli;
mod context;
mod data;
mod drafts;
mod http;
mod mcp;
mod reuse;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,chromiumoxide::handler=error")
            }),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Login => browser::login().await?,
        Command::Status => browser::status().await?,
        Command::Inspect => browser::inspect().await?,
        Command::Mcp => mcp::serve().await?,
        Command::Http(options) => http::serve(options).await?,
        Command::Logout => browser::logout()?,
    }

    Ok(())
}
