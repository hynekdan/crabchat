//! Entry point for the CrabChat client application.

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use tracing::info;
use dotenvy::dotenv;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use utils::Cli;

/// Main entry point for the client application.
///
/// This function performs the following steps:
/// 1. Initializes logging using the `tracing` crate.
/// 2. Parses command-line arguments using `clap`.
/// 3. Starts the client by calling `client::run_client`.
///
/// # Errors
/// Returns an error if the client fails to initialize or encounters issues while running.

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let username = cli.username.ok_or_else(|| {
        anyhow!(
            "Username is required. Please provide it using -u <username> or --username <username>"
        )
    })?;

    info!("Starting client for user '{}'...", username);

    client::run_client(&cli.hostname, cli.port, Some(username))
        .await
        .context("Failed to run client")?;

    info!("Client finished successfully.");
    Ok(())
}
