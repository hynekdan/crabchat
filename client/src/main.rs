use anyhow::{Context, Result, anyhow};
use clap::Parser;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use utils::Cli;

#[tokio::main]
async fn main() -> Result<()> {
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
