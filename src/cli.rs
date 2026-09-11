use std::net::{IpAddr, SocketAddr};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

pub const DEFAULT_HTTP_HOST: &str = "127.0.0.1";
pub const DEFAULT_HTTP_PORT: u16 = 3030;

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
    /// Run the MCP server over local Streamable HTTP.
    Http(HttpOptions),
    /// Remove the local browser profile after an explicit confirmation.
    Logout,
}

#[derive(Debug, Clone, Args)]
pub struct HttpOptions {
    /// Interface/IP address to bind. Defaults to loopback; non-loopback binds are explicit.
    #[arg(long, default_value = DEFAULT_HTTP_HOST)]
    pub host: String,
    /// TCP port for the MCP endpoint.
    #[arg(long, default_value_t = DEFAULT_HTTP_PORT)]
    pub port: u16,
}

impl HttpOptions {
    pub fn bind_addr(&self) -> Result<SocketAddr> {
        let host = self.host.parse::<IpAddr>().with_context(|| {
            format!("invalid HTTP bind host {:?}; use an IP address", self.host)
        })?;
        Ok(SocketAddr::new(host, self.port))
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, DEFAULT_HTTP_HOST, DEFAULT_HTTP_PORT, HttpOptions};
    use clap::Parser;

    #[test]
    fn http_defaults_to_loopback_and_safe_port() {
        let cli = Cli::try_parse_from(["fagbrev", "http"]).expect("valid CLI");
        let Command::Http(options) = cli.command else {
            panic!("expected HTTP command");
        };

        assert_eq!(options.host, DEFAULT_HTTP_HOST);
        assert_eq!(options.port, DEFAULT_HTTP_PORT);
        assert_eq!(options.bind_addr().unwrap().to_string(), "127.0.0.1:3030");
    }

    #[test]
    fn http_accepts_explicit_host_and_port() {
        let cli = Cli::try_parse_from(["fagbrev", "http", "--host", "127.0.0.2", "--port", "8765"])
            .expect("valid CLI");
        let Command::Http(options) = cli.command else {
            panic!("expected HTTP command");
        };

        assert_eq!(options.bind_addr().unwrap().to_string(), "127.0.0.2:8765");
    }

    #[test]
    fn http_rejects_non_ip_host() {
        let options = HttpOptions {
            host: "localhost".to_string(),
            port: DEFAULT_HTTP_PORT,
        };

        assert!(options.bind_addr().is_err());
    }
}
