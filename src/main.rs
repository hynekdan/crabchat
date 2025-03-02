use anyhow::{Context, Result};
use std::env;

use crabchat;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // Default settings
    let mut mode = "client";
    let mut hostname = "localhost";
    let mut port = 11111;

    // Parse command line arguments
    if args.len() > 1 && args[1] == "server" {
        mode = "server";
    }

    // Check for hostname and port args
    for i in 1..args.len() {
        if (args[i] == "--hostname" || args[i] == "-h") && i + 1 < args.len() {
            hostname = &args[i + 1];
        } else if (args[i] == "--port" || args[i] == "-p") && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse::<u16>() {
                port = p;
            }
        }
    }

    if mode == "server" {
        crabchat::listen_and_accept(hostname, port).context("Failed to run server")?;
    } else {
        crabchat::run_client(hostname, port).context("Failed to run client")?;
    }

    Ok(())
}
// TODO try to use some library for cli