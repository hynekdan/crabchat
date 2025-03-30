use anyhow::{Context, Result};
use clap::Parser;
use dotenvy::dotenv;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use utils::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    info!(
        "Attempting to start server on {}:{}...",
        cli.hostname, cli.port
    );

    server::listen_and_accept(&cli.hostname, cli.port)
        .await
        .context("Server initialization and listening failed")?;

    warn!("Server main function finished unexpectedly.");
    Ok(())
}
