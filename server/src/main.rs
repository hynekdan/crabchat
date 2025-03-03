use anyhow::{Context, Result};
use clap::Parser;

use utils::Cli;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    server::listen_and_accept(&cli.hostname, cli.port).context("Failed to run server")?;

    Ok(())
}
