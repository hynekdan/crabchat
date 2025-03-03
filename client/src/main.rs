use anyhow::{Context, Result};
use clap::Parser;

use utils::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();

    client::run_client(&cli.hostname, cli.port).context("Failed to run client")?;

    Ok(())
}
