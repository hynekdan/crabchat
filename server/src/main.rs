//! Entry point for the CrabChat server application.

use anyhow::{Context, Result};
use clap::Parser;
use dotenvy::dotenv;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use utils::Cli;

use server::ServerConfig;


/// Main entry point for the server application.
///
/// This function performs the following steps:
/// 1. Loads environment variables from a `.env` file (if present).
/// 2. Initializes logging using the `tracing` crate.
/// 3. Parses command-line arguments using `clap`.
/// 4. Starts the server by calling `server::listen_and_accept`.
///
/// # Errors
/// Returns an error if the server fails to initialize or encounters issues while running.

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    info!(
        "Attempting to start server on {}:{} with admin interface on {}:{}...",
        cli.hostname, cli.port, cli.admin_hostname, cli.admin_port
    );

    // Create server configuration
    let config = ServerConfig {
        hostname: cli.hostname,
        port: cli.port,
        admin_hostname: cli.admin_hostname,
        admin_port: cli.admin_port,
    };

    server::listen_and_accept(config)
        .await
        .context("Server initialization and listening failed")?;

    warn!("Server main function finished unexpectedly.");
    Ok(())
}
