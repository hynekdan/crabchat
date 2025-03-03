use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crabchat;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    
    // using H instead of h to avoid conflict with standard -h for help
    #[arg(short='H', long, default_value = "localhost")]
    hostname: String,
    
    #[arg(short, long, default_value_t = 11111)]
    port: u16,
}

#[derive(Subcommand)]
enum Commands {
    Server,
    Client, // default option
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Server) => {
            crabchat::listen_and_accept(&cli.hostname, cli.port)
                .context("Failed to run server")?;
        }
        Some(Commands::Client) | None => {
            // Default to client mode if no command is specified
            crabchat::run_client(&cli.hostname, cli.port)
                .context("Failed to run client")?;
        }
    }
    
    Ok(())
}
