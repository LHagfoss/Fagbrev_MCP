use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "fagbrev", about = "Local tools for a Fagbrev.io dashboard")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open a dedicated browser profile and wait for a normal Fagbrev login.
    Login,
    /// Read and print the dashboard overview.
    Status,
    /// Print sanitized page data while developing the browser adapter.
    Inspect,
    /// Run the MCP server over stdin/stdout.
    Mcp,
    /// Remove the local browser profile after an explicit confirmation.
    Logout,
}
